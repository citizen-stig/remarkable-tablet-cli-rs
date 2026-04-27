use std::time::Duration;

use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::net::TcpStream;
use tokio::time::timeout;
use uuid::Uuid;

use crate::cli::GlobalOptions;
use crate::config::ResolvedConfig;
use crate::connection::{ConnectOptions, SshConnection};
use crate::error::CliError;
pub use crate::metadata::ItemKind;
use crate::metadata::{DocumentEntry, FileType};
use crate::output;
use crate::tablet::{self};
use crate::tree::DocumentTree;

// serde's `skip_serializing_if` predicate requires `&T` by value contract.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(b: &bool) -> bool {
    !*b
}

/// Shared output-side projection of a [`DocumentEntry`]: the fields that
/// `ls`, `find`, and `info` all want in their JSON. Composed into each
/// command's output struct via `#[serde(flatten)]`. Tree mode uses its own
/// recursive shape and does not embed this.
#[derive(Serialize, Debug)]
pub struct EntryView {
    pub uuid: Uuid,
    pub name: String,
    pub path: String,
    #[serde(flatten)]
    pub kind: ItemKind,
    pub parent_uuid: Option<Uuid>,
    pub last_modified: DateTime<Utc>,
    pub last_opened: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "is_false", default)]
    pub pinned: bool,
    #[serde(skip_serializing_if = "is_false", default)]
    pub deleted: bool,
}

impl EntryView {
    #[must_use]
    pub fn from_entry(tree: &DocumentTree, entry: &DocumentEntry) -> Self {
        Self {
            uuid: entry.uuid,
            name: entry.visible_name.clone(),
            path: tree.display_path(&entry.uuid).map_or_else(
                || format!("/{}", entry.visible_name),
                std::string::ToString::to_string,
            ),
            kind: entry.kind.clone(),
            parent_uuid: entry.parent_uuid(),
            last_modified: entry.last_modified,
            last_opened: entry.last_opened,
            tags: entry.tags.clone(),
            pinned: entry.pinned,
            deleted: entry.is_trashed(),
        }
    }
}

/// Single-word label for an entry's kind. Used by `ls` flat output and `find`.
#[must_use]
pub fn type_label(kind: &ItemKind) -> &'static str {
    match kind {
        ItemKind::Folder => "folder",
        ItemKind::Document {
            file_type: FileType::Pdf,
            ..
        } => "pdf",
        ItemKind::Document {
            file_type: FileType::Epub,
            ..
        } => "epub",
        ItemKind::Document {
            file_type: FileType::Notebook,
            ..
        } => "notebook",
        ItemKind::Template => "template",
    }
}

/// File-type-only label, for callers that render kind and file type separately
/// (e.g., `info`'s human output).
#[must_use]
pub fn file_type_label(file_type: FileType) -> &'static str {
    match file_type {
        FileType::Pdf => "pdf",
        FileType::Epub => "epub",
        FileType::Notebook => "notebook",
    }
}

#[derive(Debug, Clone)]
pub struct CommandContext {
    global: GlobalOptions,
    config: ResolvedConfig,
}

pub struct ConnectedSession {
    pub ssh: SshConnection,
    pub host: String,
}

impl CommandContext {
    #[must_use]
    pub fn new(global: GlobalOptions, config: ResolvedConfig) -> Self {
        Self { global, config }
    }

    #[must_use]
    pub fn format(&self) -> output::OutputFormat {
        self.config.format
    }

    #[must_use]
    pub fn data_dir(&self) -> &str {
        &self.config.data_dir
    }

    pub fn log_verbose(&self, msg: &str) {
        output::log_verbose(&self.global, msg);
    }

    /// # Errors
    /// Returns an error if host discovery, SSH authentication, or SFTP subsystem startup fails.
    pub async fn connect(&self) -> anyhow::Result<ConnectedSession> {
        let host = discover_host(&self.global, &self.config).await?;

        self.log_verbose(&format!("connecting to {host}:{}", self.config.port));

        let opts = ConnectOptions {
            user: self.config.user.clone(),
            password: self.config.password.clone(),
            key_file: Some(self.config.key_file.clone()),
            timeout: self.config.timeout,
            verbose: self.config.verbose && !self.config.quiet,
        };

        let ssh = SshConnection::connect(&host, self.config.port, &opts)
            .await
            .context("ssh connect")?;

        Ok(ConnectedSession { ssh, host })
    }

    /// # Errors
    /// Returns an error if any `.metadata` file cannot be read or parsed.
    pub async fn load_tree(&self, ssh: &SshConnection) -> anyhow::Result<DocumentTree> {
        self.log_verbose(&format!("loading metadata from {}", self.config.data_dir));
        let (entries, diag) = tablet::load_all_metadata_full(ssh, &self.config.data_dir)
            .await
            .context("load metadata")?;
        self.log_verbose(
            &format!(
                "xochitl: {} dir entries ({}ms list_dir), {} matched <uuid>.metadata, {} parsed in {}ms, {} parse failures, {} content failures",
                diag.dir_entry_count,
                diag.list_dir_elapsed.as_millis(),
                diag.uuid_metadata_count,
                entries.len(),
                diag.read_elapsed.as_millis(),
                diag.parse_failures.len(),
                diag.content_failures.len(),
            ),
        );
        for (file, err) in &diag.parse_failures {
            self.log_verbose(&format!("  parse failed: {file}: {err}"));
        }
        for (uuid, err) in &diag.content_failures {
            self.log_verbose(&format!("  content failed: {uuid}: {err}"));
        }
        Ok(DocumentTree::build(entries))
    }

    /// # Errors
    /// Returns an error if connection fails or metadata loading fails.
    pub async fn connect_and_load_tree(&self) -> anyhow::Result<(ConnectedSession, DocumentTree)> {
        let session = self.connect().await?;
        let tree = self.load_tree(&session.ssh).await?;
        Ok((session, tree))
    }
}

const USB_HOST: &str = "10.11.99.1";
const USB_PORT: u16 = 22;
const USB_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Downcast an `anyhow::Error` to `CliError`, falling back to `IoError` so
/// any unstructured failure still produces a usable JSON envelope.
#[must_use]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{FileType, ItemKind, Parent};
    use crate::tree::DocumentTree;
    use chrono::TimeZone;

    fn make_doc(uuid: &str, name: &str, parent: Parent, file_type: FileType) -> DocumentEntry {
        let deleted = parent == Parent::Trash;
        DocumentEntry {
            uuid: Uuid::parse_str(uuid).unwrap(),
            visible_name: name.to_string(),
            kind: ItemKind::Document {
                file_type,
                page_count: None,
            },
            parent,
            deleted,
            pinned: false,
            last_modified: Utc.timestamp_millis_opt(1_710_000_000_000).unwrap(),
            version: 1,
            tags: vec![],
            last_opened: None,
        }
    }

    #[test]
    fn type_label_covers_all_kinds() {
        assert_eq!(type_label(&ItemKind::Folder), "folder");
        assert_eq!(
            type_label(&ItemKind::Document {
                file_type: FileType::Pdf,
                page_count: None
            }),
            "pdf"
        );
        assert_eq!(
            type_label(&ItemKind::Document {
                file_type: FileType::Epub,
                page_count: Some(10)
            }),
            "epub"
        );
        assert_eq!(
            type_label(&ItemKind::Document {
                file_type: FileType::Notebook,
                page_count: None
            }),
            "notebook"
        );
        assert_eq!(type_label(&ItemKind::Template), "template");
    }

    #[test]
    fn file_type_label_covers_all_file_types() {
        assert_eq!(file_type_label(FileType::Pdf), "pdf");
        assert_eq!(file_type_label(FileType::Epub), "epub");
        assert_eq!(file_type_label(FileType::Notebook), "notebook");
    }

    #[test]
    fn entry_view_uses_resolved_path_when_tree_knows_entry() {
        let entry = make_doc(
            "11111111-1111-1111-1111-111111111111",
            "Notes",
            Parent::Root,
            FileType::Notebook,
        );
        let tree = DocumentTree::build(vec![entry.clone()]);
        let view = EntryView::from_entry(&tree, &entry);
        assert_eq!(view.path, "/Notes");
        assert_eq!(view.name, "Notes");
        assert_eq!(view.uuid, entry.uuid);
        assert_eq!(view.parent_uuid, None);
        assert!(!view.deleted);
    }

    /// When the entry's parent chain doesn't reach root, `display_path`
    /// returns `None` and `EntryView` falls back to `/<visible_name>`.
    /// This shows up as the path for orphans surfaced via UUID-only lookups.
    #[test]
    fn entry_view_falls_back_to_synthetic_path_for_orphan() {
        let entry = make_doc(
            "22222222-2222-2222-2222-222222222222",
            "Mystery",
            Parent::Folder(Uuid::parse_str("99999999-9999-9999-9999-999999999999").unwrap()),
            FileType::Pdf,
        );
        let tree = DocumentTree::build(vec![]);
        let view = EntryView::from_entry(&tree, &entry);
        assert_eq!(view.path, "/Mystery");
    }

    #[test]
    fn entry_view_marks_trashed_entries() {
        let entry = make_doc(
            "33333333-3333-3333-3333-333333333333",
            "Old Draft",
            Parent::Trash,
            FileType::Pdf,
        );
        let tree = DocumentTree::build(vec![entry.clone()]);
        let view = EntryView::from_entry(&tree, &entry);
        assert!(view.deleted);
    }

    #[test]
    fn to_cli_error_preserves_existing_cli_error() {
        let err: anyhow::Error = CliError::NotFound("hi".into()).into();
        let cli = to_cli_error(err);
        match cli {
            CliError::NotFound(msg) => assert_eq!(msg, "hi"),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn to_cli_error_wraps_other_errors_as_io_error() {
        let err = anyhow::anyhow!("plain text error");
        let cli = to_cli_error(err);
        match cli {
            CliError::IoError(msg) => assert!(msg.contains("plain text error")),
            other => panic!("expected IoError, got {other:?}"),
        }
    }
}
