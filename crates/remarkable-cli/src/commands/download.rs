//! Implementation of `remarkable-cli download <path-or-uuid>`.
//!
//! Two shapes:
//! - PDF / ePub: writes a single file (`./<name>.<ext>` by default).
//! - Notebook: writes a directory with one `.rm` file per selected page,
//!   filenames mirroring the tablet's remote layout (`<page-uuid>.rm`).
//!
//! `--pages` is a 1-indexed selection over the page order recorded in
//! the document's `.content` JSON. Full notebook downloads fall back to
//! sorted filenames if that ordering can't be recovered, but `--pages`
//! requires readable page-order data.

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Serialize;
use uuid::Uuid;

use crate::cli::DownloadArgs;
use crate::commands::common::{self, CommandContext};
use crate::commands::notebook_pages;
use crate::error::CliError;
use crate::output::{self, OutputFormat};
use remarkable_metadata::metadata::{DocumentEntry, FileType, ItemKind};
use remarkable_metadata::page_range::PageSelection;
use remarkable_metadata::path_resolver::{self, Resolved};
use remarkable_metadata::tree::DocumentTree;
use remarkable_tablet::connection::TabletConnection;
use remarkable_tablet::transfer;

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
    let ext = file_type.extension();
    let remote = format!("{data_dir}/{}.{ext}", entry.uuid);
    let output_path = match explicit_output {
        Some(p) => p.to_path_buf(),
        None => PathBuf::from(format!("{}.{ext}", common::sanitize_name(&entry.visible_name))),
    };
    common::refuse_existing(&output_path)?;

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
        None => PathBuf::from(common::sanitize_name(&entry.visible_name)),
    };
    common::refuse_existing(&output_dir)?;

    let selected = notebook_pages::list_selected_pages(conn, data_dir, entry, pages).await?;

    tokio::fs::create_dir_all(&output_dir)
        .await
        .with_context(|| format!("create {}", output_dir.display()))?;

    let pages_dir = format!("{data_dir}/{}", entry.uuid);
    let jobs: Vec<(String, PathBuf)> = selected
        .iter()
        .map(|(_, name)| (format!("{pages_dir}/{name}"), output_dir.join(name)))
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
