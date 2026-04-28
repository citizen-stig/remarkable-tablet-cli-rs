//! Implementation of `remarkable-cli upload <files...>`.
//!
//! For each input PDF/ePub:
//! 1. Generate a UUID v4.
//! 2. Transfer the source file as `<uuid>.<ext>`.
//! 3. Write `<uuid>.content` with the file type.
//! 4. Write `<uuid>.metadata` with parent + timestamps.
//!
//! xochitl is stopped once before the first write and started once after
//! the last write (skipped under `--no-restart`). On failure the start is
//! still attempted so the tablet doesn't get left with xochitl down.

use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::Serialize;
use uuid::Uuid;

use crate::cli::UploadArgs;
use crate::commands::common::{self, CommandContext, is_false};
use crate::connection::TabletConnection;
use crate::error::CliError;
use crate::metadata::{FileType, ItemType, Parent, RawContent, RawMetadata};
use crate::output::{self, OutputFormat};
use crate::path_resolver::{self, Resolved};
use crate::tablet;
use crate::transfer;
use crate::tree::{ChildLookup, DocumentTree};

#[derive(Serialize, Debug)]
pub struct UploadOutput {
    /// `None` when uploading to root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_uuid: Option<Uuid>,
    pub parent_path: String,
    pub uploaded: Vec<UploadedDocument>,
    pub total_bytes: u64,
    #[serde(skip_serializing_if = "is_false", default)]
    pub dry_run: bool,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
}

#[derive(Serialize, Debug)]
pub struct UploadedDocument {
    pub uuid: Uuid,
    pub name: String,
    pub file_type: FileType,
    pub source_path: PathBuf,
    pub size_bytes: u64,
}

/// Per-file work item built up during validation and consumed by the write
/// loop.
struct Plan {
    uuid: Uuid,
    name: String,
    file_type: FileType,
    source_path: PathBuf,
    size_bytes: u64,
}

impl From<Plan> for UploadedDocument {
    fn from(p: Plan) -> Self {
        Self {
            uuid: p.uuid,
            name: p.name,
            file_type: p.file_type,
            source_path: p.source_path,
            size_bytes: p.size_bytes,
        }
    }
}

/// # Errors
/// Returns an error if the SSH connection fails, any input file is missing
/// or has an unsupported extension, the parent path does not resolve to a
/// folder, or any remote write fails.
pub async fn execute(ctx: &CommandContext, args: &UploadArgs) -> Result<(), CliError> {
    run(ctx, args).await.map_err(common::to_cli_error)
}

async fn run(ctx: &CommandContext, args: &UploadArgs) -> anyhow::Result<()> {
    let (session, tree) = ctx.connect_and_load_tree().await?;
    let result = run_with_conn(&session.ssh, ctx.data_dir(), &tree, args, ctx.no_restart()).await;
    session.ssh.disconnect().await;
    let out = result?;
    print_output(&out, ctx.format());
    Ok(())
}

/// Test-friendly core. Runs the full upload pipeline against any
/// [`TabletConnection`] and returns what was written.
///
/// # Errors
/// See [`execute`].
pub async fn run_with_conn<C: TabletConnection>(
    conn: &C,
    data_dir: &str,
    tree: &DocumentTree,
    args: &UploadArgs,
    no_restart: bool,
) -> anyhow::Result<UploadOutput> {
    if args.name.is_some() && args.files.len() > 1 {
        return Err(CliError::InvalidPath(
            "--name only valid when uploading a single file".to_string(),
        )
        .into());
    }

    let (parent, parent_uuid, parent_path) = resolve_parent(tree, args.parent.as_deref())?;

    let mut plans = Vec::with_capacity(args.files.len());
    for file_str in &args.files {
        plans.push(build_plan(file_str, args.name.as_deref()).await?);
    }

    let warnings: Vec<String> = plans
        .iter()
        .filter(|p| !matches!(tree.lookup_child(&parent, &p.name), ChildLookup::Missing))
        .map(|p| format!("parent already contains an entry named '{}'", p.name))
        .collect();

    let total_bytes: u64 = plans.iter().map(|p| p.size_bytes).sum();

    if args.dry_run {
        return Ok(UploadOutput {
            parent_uuid,
            parent_path,
            uploaded: plans.into_iter().map(UploadedDocument::from).collect(),
            total_bytes,
            dry_run: true,
            warnings,
        });
    }

    let uploaded = tablet::with_xochitl_stopped(conn, no_restart, || {
        perform_uploads(conn, data_dir, plans, &parent)
    })
    .await?;

    Ok(UploadOutput {
        parent_uuid,
        parent_path,
        uploaded,
        total_bytes,
        dry_run: false,
        warnings,
    })
}

async fn perform_uploads<C: TabletConnection>(
    conn: &C,
    data_dir: &str,
    plans: Vec<Plan>,
    parent: &Parent,
) -> anyhow::Result<Vec<UploadedDocument>> {
    let now = Utc::now();
    let mut out = Vec::with_capacity(plans.len());
    for plan in plans {
        out.push(upload_one(conn, data_dir, plan, parent, now).await?);
    }
    Ok(out)
}

async fn upload_one<C: TabletConnection>(
    conn: &C,
    data_dir: &str,
    plan: Plan,
    parent: &Parent,
    now: chrono::DateTime<Utc>,
) -> anyhow::Result<UploadedDocument> {
    let source_remote = format!("{data_dir}/{}.{}", plan.uuid, plan.file_type.extension());
    let content_remote = format!("{data_dir}/{}.content", plan.uuid);
    let metadata_remote = format!("{data_dir}/{}.metadata", plan.uuid);
    let cleanup_paths = vec![
        source_remote.clone(),
        content_remote.clone(),
        metadata_remote.clone(),
    ];

    let metadata = RawMetadata {
        visible_name: plan.name.clone(),
        item_type: ItemType::Document,
        parent: parent.clone(),
        deleted: false,
        pinned: false,
        last_modified: now,
        metadata_modified: Some(now),
        version: 1,
        tags: vec![],
        last_opened: None,
    };
    let content = RawContent {
        file_type: plan.file_type,
        page_count: None,
        pages: None,
    };

    let write_result: anyhow::Result<()> = async {
        transfer::upload_file(conn, &plan.source_path, &source_remote).await?;
        conn.write_file(&content_remote, &serde_json::to_vec(&content)?)
            .await?;
        conn.write_file(&metadata_remote, &serde_json::to_vec(&metadata)?)
            .await?;
        Ok(())
    }
    .await;

    match write_result {
        Ok(()) => Ok(plan.into()),
        Err(err) => match cleanup_partial_upload(conn, &cleanup_paths).await {
            Ok(()) => Err(err),
            Err(cleanup_err) => Err(err.context(format!(
                "cleanup after failed upload for {} ({}) left remote state uncertain: {cleanup_err:#}",
                plan.name, plan.uuid
            ))),
        },
    }
}

async fn cleanup_partial_upload<C: TabletConnection>(
    conn: &C,
    cleanup_paths: &[String],
) -> anyhow::Result<()> {
    let mut failures = Vec::new();
    for path in cleanup_paths.iter().rev() {
        match conn.file_exists(path).await {
            Ok(true) => {
                if let Err(err) = conn.remove_file(path).await {
                    failures.push(format!("{path}: {err:#}"));
                }
            }
            Ok(false) => {}
            Err(err) => failures.push(format!("check {path}: {err:#}")),
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "failed to remove partial remote files: {}",
            failures.join("; ")
        ))
    }
}

async fn build_plan(file_str: &str, name_override: Option<&str>) -> anyhow::Result<Plan> {
    let path = PathBuf::from(file_str);
    let file_type = classify_file_type(&path)?;
    let meta = tokio::fs::metadata(&path).await.map_err(|e| {
        CliError::NotFound(format!("local file not found: {} ({e})", path.display()))
    })?;
    if !meta.is_file() {
        return Err(
            CliError::InvalidPath(format!("not a regular file: {}", path.display())).into(),
        );
    }
    let name = match name_override {
        Some(n) => n.to_string(),
        None => path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                CliError::InvalidPath(format!(
                    "could not derive a name from path: {}",
                    path.display()
                ))
            })?,
    };
    Ok(Plan {
        uuid: Uuid::new_v4(),
        name,
        file_type,
        source_path: path,
        size_bytes: meta.len(),
    })
}

fn classify_file_type(path: &Path) -> anyhow::Result<FileType> {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("pdf") => Ok(FileType::Pdf),
        Some("epub") => Ok(FileType::Epub),
        _ => Err(CliError::FormatError(format!(
            "only PDF and ePub files are supported: {}",
            path.display()
        ))
        .into()),
    }
}

fn resolve_parent(
    tree: &DocumentTree,
    parent_arg: Option<&str>,
) -> anyhow::Result<(Parent, Option<Uuid>, String)> {
    let input = parent_arg.unwrap_or("/");
    match path_resolver::resolve(tree, input)? {
        Resolved::Root => Ok((Parent::Root, None, "/".to_string())),
        Resolved::Entry(e) => {
            if !e.is_folder() {
                return Err(CliError::InvalidPath(format!(
                    "parent must be a folder, not a {}: {}",
                    common::type_label(&e.kind),
                    e.visible_name
                ))
                .into());
            }
            if e.is_trashed() {
                return Err(CliError::InvalidPath(format!(
                    "cannot upload into trashed folder: {}",
                    e.visible_name
                ))
                .into());
            }
            let path = tree
                .display_path(&e.uuid)
                .map(str::to_string)
                .unwrap_or_else(|| format!("/{}", e.visible_name));
            Ok((Parent::Folder(e.uuid), Some(e.uuid), path))
        }
    }
}

fn print_output(out: &UploadOutput, format: OutputFormat) {
    println!("{}", format_output(out, format));
}

#[must_use]
pub fn format_output(out: &UploadOutput, format: OutputFormat) -> String {
    match format {
        OutputFormat::Json => output::render_json(out),
        OutputFormat::Human => format_human(out),
    }
}

fn format_human(o: &UploadOutput) -> String {
    let mut lines = vec![
        format!("parent:       {}", o.parent_path),
        format!(
            "uploaded:     {} file(s), {} bytes{}",
            o.uploaded.len(),
            o.total_bytes,
            if o.dry_run { " (dry-run)" } else { "" }
        ),
    ];
    for u in &o.uploaded {
        lines.push(format!(
            "  - {}  → uuid={}  {}  {} bytes",
            u.source_path.display(),
            u.uuid,
            common::file_type_label(u.file_type),
            u.size_bytes
        ));
    }
    for w in &o.warnings {
        lines.push(format!("warning: {w}"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_pdf_lowercase() {
        assert_eq!(
            classify_file_type(Path::new("doc.pdf")).unwrap(),
            FileType::Pdf
        );
    }

    #[test]
    fn classify_pdf_uppercase() {
        assert_eq!(
            classify_file_type(Path::new("DOC.PDF")).unwrap(),
            FileType::Pdf
        );
    }

    #[test]
    fn classify_epub() {
        assert_eq!(
            classify_file_type(Path::new("book.epub")).unwrap(),
            FileType::Epub
        );
    }

    #[test]
    fn classify_other_rejected() {
        let err = classify_file_type(Path::new("note.txt")).unwrap_err();
        let cli = err.downcast_ref::<CliError>().unwrap();
        assert!(matches!(cli, CliError::FormatError(_)));
    }

    #[test]
    fn classify_no_extension_rejected() {
        let err = classify_file_type(Path::new("foo")).unwrap_err();
        let cli = err.downcast_ref::<CliError>().unwrap();
        assert!(matches!(cli, CliError::FormatError(_)));
    }
}
