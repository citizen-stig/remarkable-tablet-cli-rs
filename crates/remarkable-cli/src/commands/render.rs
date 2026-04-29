//! Implementation of `remarkable-cli render <path-or-uuid>`.
//!
//! Renders notebook pages to PNG from either the tablet over SSH/SFTP or
//! a local backup tree (`--from-backup`). Both modes share the
//! parse-and-rasterize loop and differ only in their `TabletConnection`
//! impl — `BackupConnection` is a local-fs adapter at the bottom of this
//! file.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::Serialize;
use uuid::Uuid;

use crate::cli::RenderArgs;
use crate::commands::common::{self, CommandContext};
use crate::commands::notebook_pages;
use crate::error::CliError;
use crate::output::{self, OutputFormat};
use remarkable_metadata::metadata::{DocumentEntry, FileType, ItemKind};
use remarkable_metadata::path_resolver::{self, Resolved};
use remarkable_metadata::tree::DocumentTree;
use remarkable_rm::{RenderOptions, parse_page, render_page};
use remarkable_tablet::connection::{
    RemoteEntry, RemoteFileKind, RemoteMetadata, TabletConnection,
};
use remarkable_tablet::metadata_loader;

#[derive(Serialize, Debug)]
pub struct RenderOutput {
    pub uuid: Uuid,
    pub name: String,
    pub output_dir: PathBuf,
    pub width: u32,
    pub dpi: u32,
    pub pages: Vec<RenderedPage>,
}

#[derive(Serialize, Debug)]
pub struct RenderedPage {
    pub page: u32,
    pub output_path: PathBuf,
    pub height: u32,
}

/// # Errors
/// Returns an error if the source is unreachable, the path/UUID does not
/// resolve, the target is not a notebook, parsing fails, or any I/O
/// fails.
pub async fn execute(ctx: &CommandContext, args: &RenderArgs) -> Result<(), CliError> {
    run(ctx, args).await.map_err(common::to_cli_error)
}

async fn run(ctx: &CommandContext, args: &RenderArgs) -> anyhow::Result<()> {
    let out = if let Some(root) = args.from_backup.as_deref() {
        run_from_backup(root, args).await?
    } else {
        let (session, tree) = ctx.connect_and_load_tree().await?;
        let result = run_with_conn(&session.ssh, ctx.data_dir(), &tree, args).await;
        session.ssh.disconnect().await;
        result?
    };
    print_output(&out, ctx.format());
    Ok(())
}

/// Render against a local backup. Accepts either `<root>/xochitl/...`
/// or a path that points at the `xochitl` tree directly.
///
/// # Errors
/// Returns an error if the directory layout doesn't match a backup,
/// metadata loading fails, the path/UUID does not resolve, or rendering
/// fails.
pub async fn run_from_backup(
    backup_root: impl AsRef<Path>,
    args: &RenderArgs,
) -> anyhow::Result<RenderOutput> {
    let xochitl = locate_xochitl_root(backup_root.as_ref())?;
    let conn = BackupConnection;
    let data_dir = xochitl.to_string_lossy().into_owned();
    let entries = metadata_loader::load_all_metadata(&conn, &data_dir)
        .await
        .context("load metadata from backup")?;
    let tree = DocumentTree::build(entries);
    run_with_conn(&conn, &data_dir, &tree, args).await
}

/// Render a notebook against any [`TabletConnection`] and return what
/// was written.
///
/// # Errors
/// Returns an error for unresolvable paths, non-notebook targets,
/// invalid `--pages` combinations, output collisions, parse failures,
/// or I/O failures.
pub async fn run_with_conn<C: TabletConnection>(
    conn: &C,
    data_dir: &str,
    tree: &DocumentTree,
    args: &RenderArgs,
) -> anyhow::Result<RenderOutput> {
    let entry = match path_resolver::resolve(tree, &args.path_or_uuid)? {
        Resolved::Root => {
            return Err(CliError::InvalidPath(
                "cannot render root; specify a notebook".to_string(),
            )
            .into());
        }
        Resolved::Entry(e) => e,
    };
    expect_notebook(entry)?;

    let output_dir = match args.output.as_deref() {
        Some(p) => p.to_path_buf(),
        None => PathBuf::from(common::sanitize_name(&entry.visible_name)),
    };
    common::refuse_existing(&output_dir)?;

    let selected =
        notebook_pages::list_selected_pages(conn, data_dir, entry, args.pages.as_ref()).await?;

    tokio::fs::create_dir_all(&output_dir)
        .await
        .with_context(|| format!("create {}", output_dir.display()))?;

    let mut rendered = Vec::with_capacity(selected.len());
    for (page_index, page_filename) in selected {
        let remote = format!("{data_dir}/{}/{}", entry.uuid, page_filename);
        let bytes = conn
            .read_file(&remote)
            .await
            .with_context(|| format!("read {remote}"))?;
        let page = parse_page(&bytes)
            .with_context(|| format!("parse {remote}"))?;
        let opts = RenderOptions {
            width: args.width,
            height: page.paper_size.map_or(remarkable_rm::DEFAULT_HEIGHT, |(_, h)| h),
        };
        let png = render_page(&page, &opts)
            .with_context(|| format!("render {remote}"))?;
        let output_path = output_dir.join(format!("{}_page_{page_index}.png", entry.uuid));
        tokio::fs::write(&output_path, &png)
            .await
            .with_context(|| format!("write {}", output_path.display()))?;

        rendered.push(RenderedPage {
            page: page_index,
            output_path,
            height: opts.height,
        });
    }

    Ok(RenderOutput {
        uuid: entry.uuid,
        name: entry.visible_name.clone(),
        output_dir,
        width: args.width,
        dpi: args.dpi,
        pages: rendered,
    })
}

fn expect_notebook(entry: &DocumentEntry) -> anyhow::Result<()> {
    match entry.kind {
        ItemKind::Document {
            file_type: FileType::Notebook,
            ..
        } => Ok(()),
        ItemKind::Document { file_type, .. } => Err(CliError::InvalidPath(format!(
            "cannot render `{}` (file_type: {}); render only supports notebooks",
            entry.visible_name,
            common::file_type_label(file_type),
        ))
        .into()),
        ItemKind::Folder => Err(CliError::InvalidPath(format!(
            "cannot render folder `{}`",
            entry.visible_name
        ))
        .into()),
        ItemKind::Template => Err(CliError::InvalidPath(format!(
            "cannot render template `{}`",
            entry.visible_name
        ))
        .into()),
    }
}

/// Find the `xochitl` directory inside a backup root. Accepts either
/// `<root>` (containing `xochitl/`) or `<root>/xochitl` directly so users
/// can point at whichever form their `backup` command produced.
fn locate_xochitl_root(root: &Path) -> anyhow::Result<PathBuf> {
    if !root.exists() {
        bail!("backup root does not exist: {}", root.display());
    }
    let nested = root.join("xochitl");
    if nested.is_dir() {
        return Ok(nested);
    }
    if root.is_dir() {
        return Ok(root.to_path_buf());
    }
    bail!(
        "expected a backup directory at {} (with optional `xochitl/` subdir)",
        root.display()
    )
}

fn print_output(out: &RenderOutput, format: OutputFormat) {
    println!("{}", format_output(out, format));
}

#[must_use]
pub fn format_output(out: &RenderOutput, format: OutputFormat) -> String {
    match format {
        OutputFormat::Json => output::render_json(out),
        OutputFormat::Human => format_human(out),
    }
}

fn format_human(o: &RenderOutput) -> String {
    let mut lines = vec![
        format!("uuid:        {}", o.uuid),
        format!("name:        {}", o.name),
        format!("output_dir:  {}", o.output_dir.display()),
        format!("width:       {}", o.width),
        format!("dpi:         {}", o.dpi),
        format!("pages:       {}", o.pages.len()),
    ];
    for page in &o.pages {
        lines.push(format!(
            "  page {:>3}: {} ({}x{})",
            page.page,
            page.output_path.display(),
            o.width,
            page.height
        ));
    }
    lines.join("\n")
}

/// Local-filesystem [`TabletConnection`] used only for `--from-backup`.
///
/// Treats every incoming path as a literal local path. Read methods
/// delegate to `tokio::fs`; mutation/exec methods return errors because
/// rendering from a backup is a read-only operation.
struct BackupConnection;

impl TabletConnection for BackupConnection {
    async fn read_file(&self, path: &str) -> anyhow::Result<Vec<u8>> {
        tokio::fs::read(path)
            .await
            .with_context(|| format!("read {path}"))
    }

    async fn write_file(&self, _path: &str, _data: &[u8]) -> anyhow::Result<()> {
        bail!("backup connection is read-only");
    }

    async fn read_dir(&self, path: &str) -> anyhow::Result<Vec<RemoteEntry>> {
        let mut rd = tokio::fs::read_dir(path)
            .await
            .with_context(|| format!("read_dir {path}"))?;
        let mut out = Vec::new();
        while let Some(entry) = rd.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            let meta = entry.metadata().await?;
            out.push(RemoteEntry {
                name,
                metadata: into_remote_metadata(&meta),
            });
        }
        Ok(out)
    }

    async fn stat(&self, path: &str) -> anyhow::Result<RemoteMetadata> {
        let meta = tokio::fs::metadata(path)
            .await
            .with_context(|| format!("stat {path}"))?;
        Ok(into_remote_metadata(&meta))
    }

    async fn remove_file(&self, _path: &str) -> anyhow::Result<()> {
        bail!("backup connection is read-only");
    }

    async fn remove_dir_all(&self, _path: &str) -> anyhow::Result<()> {
        bail!("backup connection is read-only");
    }

    async fn execute(&self, _command: &str) -> anyhow::Result<String> {
        bail!("backup connection cannot execute commands");
    }

    async fn file_exists(&self, path: &str) -> anyhow::Result<bool> {
        match tokio::fs::metadata(path).await {
            Ok(_) => Ok(true),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err).with_context(|| format!("stat {path}")),
        }
    }
}

fn into_remote_metadata(meta: &std::fs::Metadata) -> RemoteMetadata {
    let kind = if meta.is_dir() {
        RemoteFileKind::Dir
    } else if meta.is_file() {
        RemoteFileKind::File
    } else {
        RemoteFileKind::Other
    };
    RemoteMetadata {
        size: Some(meta.len()),
        mtime: meta.modified().ok(),
        kind,
    }
}

