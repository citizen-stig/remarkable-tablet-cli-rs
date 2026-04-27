use std::time::Duration;

use anyhow::Context;
use serde::Serialize;
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::cli::GlobalOptions;
use crate::config::{self, ResolvedConfig};
use crate::connection::{ConnectOptions, SshConnection};
use crate::error::CliError;
use crate::metadata::{DocumentEntry, FileType, ItemType};
use crate::output;
use crate::path_resolver;
use crate::tablet::{self};
use crate::tree::DocumentTree;

/// User-facing projection of [`ItemType`] for the CLI's JSON output schema:
/// serializes as lowercase `"folder"`/`"document"`/`"template"` and renames
/// `Collection` → `Folder` to match how users describe a tablet's hierarchy.
///
/// Kept separate from `ItemType` because serde tags apply equally in both
/// directions — one enum can't serialize as `"DocumentType"` for
/// `.metadata` round-trips and `"document"` for CLI output. Convert via
/// `ItemKind::from(item_type)` at the output boundary.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ItemKind {
    Folder,
    Document,
    Template,
}

impl From<ItemType> for ItemKind {
    fn from(t: ItemType) -> Self {
        match t {
            ItemType::Collection => ItemKind::Folder,
            ItemType::Document => ItemKind::Document,
            ItemType::Template => ItemKind::Template,
        }
    }
}

/// Combined label for `(kind, file_type)`. Used by `ls` flat output and `find`.
pub fn type_label(kind: ItemKind, file_type: Option<FileType>) -> &'static str {
    match (kind, file_type) {
        (ItemKind::Folder, _) => "folder",
        (ItemKind::Document, Some(FileType::Pdf)) => "pdf",
        (ItemKind::Document, Some(FileType::Epub)) => "epub",
        (ItemKind::Document, Some(FileType::Notebook)) => "notebook",
        (ItemKind::Document, None) => "document",
        (ItemKind::Template, _) => "template",
    }
}

/// File-type-only label, for callers that render kind and file type separately
/// (e.g., `info`'s human output).
pub fn file_type_label(file_type: FileType) -> &'static str {
    match file_type {
        FileType::Pdf => "pdf",
        FileType::Epub => "epub",
        FileType::Notebook => "notebook",
    }
}

/// Resolve `entry`'s full path, falling back to a top-level synthetic path on
/// resolver failure (broken parent chain or missing intermediate folder).
pub fn entry_path(tree: &DocumentTree, entry: &DocumentEntry) -> String {
    path_resolver::resolve_uuid_to_path(tree, &entry.uuid)
        .unwrap_or_else(|_| format!("/{}", entry.visible_name))
}

const USB_HOST: &str = "10.11.99.1";
const USB_PORT: u16 = 22;
const USB_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Resolve config, discover the tablet host, and open an SSH session.
///
/// Returns the live `SshConnection` plus the merged config so callers can
/// reuse derived values (data_dir, format, etc.). Caller is responsible for
/// `ssh.disconnect().await` when finished.
pub async fn connect(global: &GlobalOptions) -> anyhow::Result<(SshConnection, ResolvedConfig)> {
    let file_cfg = config::load_file_config(None).unwrap_or_default();
    let mut resolved = config::resolve(global, &file_cfg);
    let host = discover_host(global, &resolved).await?;

    output::log_verbose(global, &format!("connecting to {host}:{}", resolved.port));

    let opts = ConnectOptions {
        user: resolved.user.clone(),
        password: resolved.password.clone(),
        key_file: Some(resolved.key_file.clone()),
        timeout: resolved.timeout,
        verbose: resolved.verbose && !resolved.quiet,
    };

    let ssh = SshConnection::connect(&host, resolved.port, &opts)
        .await
        .context("ssh connect")?;

    resolved.host = Some(host);
    Ok((ssh, resolved))
}

/// Connect, then load the full document tree from the tablet.
///
/// Convenience for read-only browse commands. Caller is responsible for
/// `ssh.disconnect().await` when finished with the connection.
pub async fn connect_and_load_tree(
    global: &GlobalOptions,
) -> anyhow::Result<(SshConnection, ResolvedConfig, DocumentTree)> {
    let (ssh, cfg) = connect(global).await?;
    output::log_verbose(global, &format!("loading metadata from {}", cfg.data_dir));
    let (entries, diag) = tablet::load_all_metadata_full(&ssh, &cfg.data_dir)
        .await
        .context("load metadata")?;
    output::log_verbose(
        global,
        &format!(
            "xochitl: {} dir entries ({}ms list_dir), {} matched <uuid>.metadata, {} parsed in {}ms, {} parse failures",
            diag.dir_entry_count,
            diag.list_dir_elapsed.as_millis(),
            diag.uuid_metadata_count,
            entries.len(),
            diag.read_elapsed.as_millis(),
            diag.parse_failures.len(),
        ),
    );
    for (file, err) in &diag.parse_failures {
        output::log_verbose(global, &format!("  parse failed: {file}: {err}"));
    }
    Ok((ssh, cfg, DocumentTree::build(entries)))
}

/// Downcast an `anyhow::Error` to `CliError`, falling back to `IoError` so
/// any unstructured failure still produces a usable JSON envelope.
pub fn to_cli_error(err: anyhow::Error) -> CliError {
    match err.downcast::<CliError>() {
        Ok(cli) => cli,
        Err(other) => CliError::IoError(format!("{other:#}")),
    }
}

async fn discover_host(global: &GlobalOptions, cfg: &ResolvedConfig) -> anyhow::Result<String> {
    if let Some(h) = cfg.host.as_deref() {
        return Ok(h.to_string());
    }
    output::log_verbose(
        global,
        &format!("auto-discover: probing USB fallback {USB_HOST}:{USB_PORT}"),
    );
    let probe = timeout(
        USB_PROBE_TIMEOUT,
        TcpStream::connect(format!("{USB_HOST}:{USB_PORT}")),
    )
    .await;
    match probe {
        Ok(Ok(_)) => Ok(USB_HOST.to_string()),
        _ => Err(anyhow::Error::new(CliError::ConnectionFailed(
            "Could not auto-discover tablet. Connect via USB (10.11.99.1) or pass --host. \
             You can also set the host in ~/.config/remarkable-cli/config.toml."
                .to_string(),
        ))),
    }
}
