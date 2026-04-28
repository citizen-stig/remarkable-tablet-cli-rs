//! Implementation of `remarkable-cli upload <files...>`.
//!
//! For each input PDF/ePub:
//! 1. Generate a UUID v4.
//! 2. Write `<uuid>.metadata` with parent + timestamps.
//! 3. Write `<uuid>.content` with the file type.
//! 4. Transfer the source file as `<uuid>.<ext>`.
//!
//! xochitl is stopped once before the first write and started once after
//! the last write (skipped under `--no-restart`). On failure the start is
//! still attempted so the tablet doesn't get left with xochitl down.

use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::Serialize;
use uuid::Uuid;

use crate::cli::UploadArgs;
use crate::commands::common::{self, CommandContext};
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

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(b: &bool) -> bool {
    !*b
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

/// # Errors
/// Returns an error if the SSH connection fails, any input file is missing
/// or has an unsupported extension, the parent path does not resolve to a
/// folder, or any remote write fails.
pub async fn execute(ctx: &CommandContext, args: &UploadArgs) -> Result<(), CliError> {
    run(ctx, args).await.map_err(common::to_cli_error)
}

async fn run(ctx: &CommandContext, args: &UploadArgs) -> anyhow::Result<()> {
    let (session, tree) = ctx.connect_and_load_tree().await?;
    let result =
        run_with_conn(&session.ssh, ctx.data_dir(), &tree, args, ctx.no_restart()).await;
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
        .filter_map(|p| match tree.lookup_child(&parent, &p.name) {
            ChildLookup::Missing => None,
            _ => Some(format!(
                "parent already contains an entry named '{}'",
                p.name
            )),
        })
        .collect();

    let total_bytes: u64 = plans.iter().map(|p| p.size_bytes).sum();

    if args.dry_run {
        return Ok(UploadOutput {
            parent_uuid,
            parent_path,
            uploaded: plans.into_iter().map(plan_to_uploaded).collect(),
            total_bytes,
            dry_run: true,
            warnings,
        });
    }

    tablet::stop_xochitl(conn).await?;

    let upload_result = perform_uploads(conn, data_dir, &plans, &parent).await;

    // Always try to bring xochitl back so the tablet doesn't sit with the
    // service down. Upload errors take precedence over restart errors —
    // they're the user's primary failure to act on.
    let restart_result = if no_restart {
        Ok(())
    } else {
        tablet::start_xochitl(conn).await
    };

    let uploaded = upload_result?;
    restart_result?;

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
    plans: &[Plan],
    parent: &Parent,
) -> anyhow::Result<Vec<UploadedDocument>> {
    let now = Utc::now();
    let mut out = Vec::with_capacity(plans.len());
    for plan in plans {
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
        let metadata_bytes = serde_json::to_vec(&metadata)?;
        conn.write_file(
            &format!("{data_dir}/{}.metadata", plan.uuid),
            &metadata_bytes,
        )
        .await?;

        let content = RawContent {
            file_type: plan.file_type,
            page_count: None,
            pages: None,
        };
        let content_bytes = serde_json::to_vec(&content)?;
        conn.write_file(
            &format!("{data_dir}/{}.content", plan.uuid),
            &content_bytes,
        )
        .await?;

        let ext = source_extension(plan.file_type);
        let written = transfer::upload_file(
            conn,
            &plan.source_path,
            &format!("{data_dir}/{}.{ext}", plan.uuid),
        )
        .await?;
        debug_assert_eq!(written, plan.size_bytes);

        out.push(plan_to_uploaded_ref(plan));
    }
    Ok(out)
}

fn plan_to_uploaded(p: Plan) -> UploadedDocument {
    UploadedDocument {
        uuid: p.uuid,
        name: p.name,
        file_type: p.file_type,
        source_path: p.source_path,
        size_bytes: p.size_bytes,
    }
}

fn plan_to_uploaded_ref(p: &Plan) -> UploadedDocument {
    UploadedDocument {
        uuid: p.uuid,
        name: p.name.clone(),
        file_type: p.file_type,
        source_path: p.source_path.clone(),
        size_bytes: p.size_bytes,
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

fn source_extension(file_type: FileType) -> &'static str {
    match file_type {
        FileType::Pdf => "pdf",
        FileType::Epub => "epub",
        FileType::Notebook => "rm",
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
