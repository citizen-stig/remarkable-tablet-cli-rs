//! Implementation of `remarkable-cli read <path-or-uuid>`.
//!
//! One-step "give me the consumable form of this document" command, so an
//! agent (or human) doesn't need to know the raw-`.rm`-vs-render distinction:
//!
//! - Notebook -> render its pages to PNG (delegates to [`crate::commands::render`]).
//! - PDF / ePub -> download the source file (delegates to [`crate::commands::download`]).
//!
//! The raw `.rm` page files are still reachable via `download` for callers that
//! want them; `read` deliberately picks the rendered form for notebooks.

use std::path::PathBuf;

use serde::Serialize;
use uuid::Uuid;

use crate::cli::{DownloadArgs, ReadArgs, RenderArgs};
use crate::commands::common::{self, CommandContext};
use crate::commands::download::{self, DownloadOutput};
use crate::commands::render::{self, RenderOutput};
use crate::error::CliError;
use crate::output::{self, OutputFormat};
use remarkable_metadata::metadata::{FileType, ItemKind};
use remarkable_metadata::path_resolver::{self, Resolved};
use remarkable_metadata::tree::DocumentTree;
use remarkable_tablet::connection::TabletConnection;

#[derive(Serialize, Debug)]
pub struct ReadOutput {
    pub uuid: Uuid,
    pub name: String,
    pub file_type: FileType,
    /// `"rendered"` for notebooks (PNG pages), `"downloaded"` for PDF/ePub.
    pub action: &'static str,
    /// Directory of PNGs (notebooks) or the source file (PDF/ePub).
    pub output_path: PathBuf,
    /// One entry per artifact written, in order.
    pub artifacts: Vec<ReadArtifact>,
    /// Total bytes written. Omitted for renders (PNG sizes aren't tracked).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

#[derive(Serialize, Debug)]
pub struct ReadArtifact {
    pub path: PathBuf,
    /// 1-indexed page number for notebook PNGs; absent for PDF/ePub.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
}

impl ReadOutput {
    fn from_render(r: RenderOutput) -> Self {
        let artifacts = r
            .pages
            .into_iter()
            .map(|p| ReadArtifact {
                path: p.output_path,
                page: Some(p.page),
            })
            .collect();
        Self {
            uuid: r.uuid,
            name: r.name,
            file_type: FileType::Notebook,
            action: "rendered",
            output_path: r.output_dir,
            artifacts,
            size_bytes: None,
        }
    }

    fn from_download(d: DownloadOutput) -> Self {
        Self {
            uuid: d.uuid,
            name: d.name,
            file_type: d.file_type,
            action: "downloaded",
            artifacts: vec![ReadArtifact {
                path: d.output_path.clone(),
                page: None,
            }],
            output_path: d.output_path,
            size_bytes: Some(d.size_bytes),
        }
    }
}

/// # Errors
/// Returns an error if the SSH connection fails, the path/UUID does not
/// resolve, the target is a folder/template, or the underlying render/download
/// fails.
pub async fn execute(ctx: &CommandContext, args: &ReadArgs) -> Result<(), CliError> {
    run(ctx, args).await.map_err(common::to_cli_error)
}

async fn run(ctx: &CommandContext, args: &ReadArgs) -> anyhow::Result<()> {
    let (session, tree) = ctx.connect_and_load_tree().await?;
    let result = run_with_conn(&session.ssh, ctx.data_dir(), &tree, args).await;
    session.ssh.disconnect().await;
    let out = result?;
    print_output(&out, ctx.format());
    Ok(())
}

/// Test-friendly core: resolve the target, pick render vs download by file
/// type, and return the unified result.
///
/// # Errors
/// Returns an error for unresolvable paths, non-document targets, invalid
/// `--pages` combinations, output collisions, or I/O failures.
pub async fn run_with_conn<C: TabletConnection>(
    conn: &C,
    data_dir: &str,
    tree: &DocumentTree,
    args: &ReadArgs,
) -> anyhow::Result<ReadOutput> {
    let entry = match path_resolver::resolve(tree, &args.path_or_uuid)? {
        Resolved::Root => {
            return Err(
                CliError::InvalidPath("cannot read root; specify a document".to_string()).into(),
            );
        }
        Resolved::Entry(e) => e,
    };

    let file_type = match entry.kind {
        ItemKind::Document { file_type, .. } => file_type,
        ItemKind::Folder => {
            return Err(CliError::InvalidPath(format!(
                "cannot read folder `{}`",
                entry.visible_name
            ))
            .into());
        }
        ItemKind::Template => {
            return Err(CliError::InvalidPath(format!(
                "cannot read template `{}`",
                entry.visible_name
            ))
            .into());
        }
    };

    match file_type {
        FileType::Notebook => {
            let render_args = RenderArgs {
                path_or_uuid: args.path_or_uuid.clone(),
                output: args.output.clone(),
                pages: args.pages.clone(),
                width: remarkable_rm::DEFAULT_WIDTH,
                dpi: 226,
                from_backup: None,
            };
            let rendered = render::run_with_conn(conn, data_dir, tree, &render_args).await?;
            Ok(ReadOutput::from_render(rendered))
        }
        FileType::Pdf | FileType::Epub => {
            if args.pages.is_some() {
                return Err(CliError::InvalidPath(
                    "--pages is only valid for notebooks".to_string(),
                )
                .into());
            }
            let download_args = DownloadArgs {
                path_or_uuid: args.path_or_uuid.clone(),
                output: args.output.clone(),
                pages: None,
            };
            let downloaded = download::run_with_conn(conn, data_dir, tree, &download_args).await?;
            Ok(ReadOutput::from_download(downloaded))
        }
    }
}

fn print_output(out: &ReadOutput, format: OutputFormat) {
    println!("{}", format_output(out, format));
}

#[must_use]
pub fn format_output(out: &ReadOutput, format: OutputFormat) -> String {
    match format {
        OutputFormat::Json => output::render_json(out),
        OutputFormat::Human => format_human(out),
    }
}

fn format_human(o: &ReadOutput) -> String {
    let mut lines = vec![
        format!("uuid:         {}", o.uuid),
        format!("name:         {}", o.name),
        format!("file_type:    {}", common::file_type_label(o.file_type)),
        format!("action:       {}", o.action),
        format!("output_path:  {}", o.output_path.display()),
    ];
    if let Some(bytes) = o.size_bytes {
        lines.push(format!("size_bytes:   {bytes}"));
    }
    lines.push(format!("artifacts:    {}", o.artifacts.len()));
    for a in &o.artifacts {
        match a.page {
            Some(page) => lines.push(format!("  page {page:>3}: {}", a.path.display())),
            None => lines.push(format!("  - {}", a.path.display())),
        }
    }
    lines.join("\n")
}
