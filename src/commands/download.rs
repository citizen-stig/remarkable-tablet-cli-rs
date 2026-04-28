//! Implementation of `remarkable-cli download <path-or-uuid>`.
//!
//! Two shapes:
//! - PDF / ePub: writes a single file (`./<name>.<ext>` by default).
//! - Notebook: writes a directory with one `.rm` file per selected page,
//!   filenames mirroring the tablet's remote layout (`<page-uuid>.rm`).
//!
//! `--pages` is a 1-indexed selection over the page order recorded in
//! the document's `.content` JSON; if that ordering can't be recovered
//! we fall back to sorted filename order.

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Serialize;
use uuid::Uuid;

use crate::cli::DownloadArgs;
use crate::commands::common::{self, CommandContext};
use crate::connection::TabletConnection;
use crate::error::CliError;
use crate::metadata::{DocumentEntry, FileType, ItemKind};
use crate::output::{self, OutputFormat};
use crate::page_range::PageSelection;
use crate::path_resolver::{self, Resolved};
use crate::transfer;
use crate::tree::DocumentTree;

#[derive(Serialize, Debug)]
pub struct DownloadOutput {
    pub uuid: Uuid,
    pub name: String,
    pub file_type: FileType,
    pub output_path: PathBuf,
    pub size_bytes: u64,
    /// Notebooks only: number of `.rm` files written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pages_written: Option<usize>,
}

/// # Errors
/// Returns an error if the SSH connection fails, the path/UUID does
/// not resolve, the target is not a downloadable document, or any file
/// I/O fails.
pub async fn execute(ctx: &CommandContext, args: &DownloadArgs) -> Result<(), CliError> {
    run(ctx, args).await.map_err(common::to_cli_error)
}

async fn run(ctx: &CommandContext, args: &DownloadArgs) -> anyhow::Result<()> {
    let (session, tree) = ctx.connect_and_load_tree().await?;
    let result = run_with_conn(&session.ssh, ctx.data_dir(), &tree, args).await;
    session.ssh.disconnect().await;
    let out = result?;
    print_output(&out, ctx.format());
    Ok(())
}

/// Test-friendly core: perform the download against any
/// `TabletConnection` and return what was written.
///
/// # Errors
/// Returns an error for unresolvable paths, non-document targets,
/// invalid `--pages` combinations, output collisions, or I/O failures.
pub async fn run_with_conn<C: TabletConnection>(
    conn: &C,
    data_dir: &str,
    tree: &DocumentTree,
    args: &DownloadArgs,
) -> anyhow::Result<DownloadOutput> {
    let entry = match path_resolver::resolve(tree, &args.path_or_uuid)? {
        Resolved::Root => {
            return Err(CliError::InvalidPath(
                "cannot download root; specify a document".to_string(),
            )
            .into());
        }
        Resolved::Entry(e) => e,
    };

    let file_type = match entry.kind {
        ItemKind::Document { file_type, .. } => file_type,
        ItemKind::Folder => {
            return Err(CliError::InvalidPath(format!(
                "cannot download folder `{}`",
                entry.visible_name
            ))
            .into());
        }
        ItemKind::Template => {
            return Err(CliError::InvalidPath(format!(
                "cannot download template `{}`",
                entry.visible_name
            ))
            .into());
        }
    };

    if args.pages.is_some() && file_type != FileType::Notebook {
        return Err(
            CliError::InvalidPath("--pages is only valid for notebooks".to_string()).into(),
        );
    }

    match file_type {
        FileType::Pdf | FileType::Epub => {
            download_source_file(conn, data_dir, entry, file_type, args.output.as_deref()).await
        }
        FileType::Notebook => {
            download_notebook(
                conn,
                data_dir,
                entry,
                args.pages.as_ref(),
                args.output.as_deref(),
            )
            .await
        }
    }
}

async fn download_source_file<C: TabletConnection>(
    conn: &C,
    data_dir: &str,
    entry: &DocumentEntry,
    file_type: FileType,
    explicit_output: Option<&Path>,
) -> anyhow::Result<DownloadOutput> {
    let ext = source_extension(file_type);
    let remote = format!("{data_dir}/{}.{ext}", entry.uuid);
    let output_path = match explicit_output {
        Some(p) => p.to_path_buf(),
        None => PathBuf::from(format!("{}.{ext}", sanitize_name(&entry.visible_name))),
    };
    refuse_existing(&output_path)?;

    let written = transfer::download_file(conn, &remote, &output_path)
        .await
        .with_context(|| format!("download {remote}"))?;

    Ok(DownloadOutput {
        uuid: entry.uuid,
        name: entry.visible_name.clone(),
        file_type,
        output_path,
        size_bytes: written,
        pages_written: None,
    })
}

async fn download_notebook<C: TabletConnection>(
    conn: &C,
    data_dir: &str,
    entry: &DocumentEntry,
    pages: Option<&PageSelection>,
    explicit_output: Option<&Path>,
) -> anyhow::Result<DownloadOutput> {
    let output_dir = match explicit_output {
        Some(p) => p.to_path_buf(),
        None => PathBuf::from(sanitize_name(&entry.visible_name)),
    };
    refuse_existing(&output_dir)?;

    let pages_dir = format!("{data_dir}/{}", entry.uuid);
    // Discover all `.rm` files. If the page directory itself is missing
    // (notebook has no recorded pages yet), treat as zero pages rather
    // than an error.
    let entries = conn.read_dir(&pages_dir).await.unwrap_or_default();
    let mut page_files: Vec<String> = entries
        .into_iter()
        .filter(|e| {
            std::path::Path::new(&e.name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("rm"))
        })
        .map(|e| e.name)
        .collect();

    let ordered = order_page_files(conn, data_dir, entry.uuid, &mut page_files).await;

    let selected: Vec<&String> = match pages {
        Some(sel) => ordered
            .iter()
            .enumerate()
            .filter_map(|(idx, name)| {
                let one_based = u32::try_from(idx + 1).ok()?;
                if sel.contains(one_based) {
                    Some(name)
                } else {
                    None
                }
            })
            .collect(),
        None => ordered.iter().collect(),
    };

    tokio::fs::create_dir_all(&output_dir)
        .await
        .with_context(|| format!("create {}", output_dir.display()))?;

    let jobs: Vec<(String, PathBuf)> = selected
        .iter()
        .map(|name| {
            (
                format!("{pages_dir}/{name}"),
                output_dir.join(name.as_str()),
            )
        })
        .collect();
    let pages_count = jobs.len();
    let total_bytes = transfer::download_many(conn, jobs)
        .await
        .context("download notebook pages")?;

    Ok(DownloadOutput {
        uuid: entry.uuid,
        name: entry.visible_name.clone(),
        file_type: FileType::Notebook,
        output_path: output_dir,
        size_bytes: total_bytes,
        pages_written: Some(pages_count),
    })
}

/// Try to recover the page order recorded in the document's `.content`
/// file; fall back to sorted filename order when the JSON shape isn't
/// understood. The returned vector contains every `.rm` filename — any
/// pages listed in `.content` but missing on disk are silently dropped,
/// and any orphan `.rm` files not referenced by `.content` are appended
/// at the end.
async fn order_page_files<C: TabletConnection>(
    conn: &C,
    data_dir: &str,
    uuid: Uuid,
    discovered: &mut Vec<String>,
) -> Vec<String> {
    discovered.sort();
    let content_path = format!("{data_dir}/{uuid}.content");
    let Ok(bytes) = conn.read_file(&content_path).await else {
        return std::mem::take(discovered);
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return std::mem::take(discovered);
    };
    let Some(pages_value) = value.get("pages") else {
        return std::mem::take(discovered);
    };

    let mut ordered = Vec::with_capacity(discovered.len());
    let mut remaining: std::collections::HashSet<String> = discovered.iter().cloned().collect();
    if let Some(arr) = pages_value.as_array() {
        for item in arr {
            let Some(page_id) = extract_page_id(item) else {
                continue;
            };
            let filename = format!("{page_id}.rm");
            if remaining.remove(&filename) {
                ordered.push(filename);
            }
        }
    }
    let mut leftover: Vec<String> = remaining.into_iter().collect();
    leftover.sort();
    ordered.extend(leftover);
    ordered
}

fn extract_page_id(item: &serde_json::Value) -> Option<String> {
    if let Some(s) = item.as_str() {
        return Some(s.to_string());
    }
    if let Some(obj) = item.as_object() {
        if let Some(id) = obj.get("id").and_then(|v| v.as_str()) {
            return Some(id.to_string());
        }
        if let Some(id) = obj.get("uuid").and_then(|v| v.as_str()) {
            return Some(id.to_string());
        }
    }
    None
}

fn refuse_existing(path: &Path) -> anyhow::Result<()> {
    if path.exists() {
        return Err(CliError::AlreadyExists(format!(
            "output path already exists: {}",
            path.display()
        ))
        .into());
    }
    Ok(())
}

fn source_extension(file_type: FileType) -> &'static str {
    match file_type {
        FileType::Pdf => "pdf",
        FileType::Epub => "epub",
        FileType::Notebook => "rm", // unused — handled separately
    }
}

/// Replace path-unsafe characters in a `visibleName` so it can be used
/// as a filename component. Currently just `/` since that's the only
/// reMarkable-allowed character that breaks local paths; conservative
/// callers can also pass `--output` to skip this entirely.
fn sanitize_name(name: &str) -> String {
    name.replace('/', "_")
}

fn print_output(out: &DownloadOutput, format: OutputFormat) {
    println!("{}", format_output(out, format));
}

#[must_use]
pub fn format_output(out: &DownloadOutput, format: OutputFormat) -> String {
    match format {
        OutputFormat::Json => output::render_json(out),
        OutputFormat::Human => format_human(out),
    }
}

fn format_human(o: &DownloadOutput) -> String {
    let mut lines = vec![
        format!("uuid:         {}", o.uuid),
        format!("name:         {}", o.name),
        format!("file_type:    {}", common::file_type_label(o.file_type)),
        format!("output_path:  {}", o.output_path.display()),
        format!("size_bytes:   {}", o.size_bytes),
    ];
    if let Some(pages) = o.pages_written {
        lines.push(format!("pages_written: {pages}"));
    }
    lines.join("\n")
}
