use std::time::Duration;

/// Source of a filesystem-shaped failure surfaced via [`TabletError::Io`]. A
/// closed enum so callers can pattern-match the backend without losing the
/// concrete error type — no `Box<dyn Error>`.
#[derive(Debug, thiserror::Error)]
pub enum IoSource {
    /// SFTP protocol error from the real tablet connection.
    #[error(transparent)]
    Sftp(#[from] russh_sftp::client::error::Error),

    /// `std::io::Error` from the test [`crate::connection::FakeConnection`],
    /// the CLI's read-only backup-tree shim, or `transfer.rs`'s local-fs
    /// operations. Also covers SFTP `AsyncWrite` errors (`write_all` /
    /// `shutdown`) which surface through tokio as `io::Error` even though
    /// the underlying transport is SFTP.
    #[error(transparent)]
    Local(#[from] std::io::Error),
}

/// Errors produced by the tablet client (connection, SFTP, xochitl control,
/// local file transfer, metadata loading).
#[derive(Debug, thiserror::Error)]
pub enum TabletError {
    #[error("connect to {addr} timed out after {timeout:?}")]
    ConnectTimeout { addr: String, timeout: Duration },

    #[error("connect to {addr}: {source}")]
    Connect {
        addr: String,
        #[source]
        source: russh::Error,
    },

    #[error("authentication failed: {0}")]
    AuthFailed(String),

    /// Filesystem-shaped operation against the tablet data tree, regardless of
    /// transport. `op` is one of `"read" | "write" | "create" | "close" |
    /// "stat" | "read_dir" | "remove_file" | "remove_dir" | "create_dir" |
    /// "exists" | "start_session"`. The [`IoSource`] discriminates the
    /// concrete backend error.
    #[error("io {op} {path}: {source}")]
    Io {
        op: &'static str,
        path: String,
        #[source]
        source: IoSource,
    },

    /// SSH-level operation (channel open, exec, sftp-subsystem request).
    /// `op` is free-form and includes contextual detail such as the wrapped
    /// command name.
    #[error("ssh {op}: {source}")]
    Ssh {
        op: String,
        #[source]
        source: russh::Error,
    },

    #[error("command output not UTF-8: {source}")]
    CommandOutputNotUtf8 {
        #[source]
        source: std::string::FromUtf8Error,
    },

    /// Output of a remote shell command did not match the expected shape
    /// (e.g. `df -k` produced fewer fields than a row).
    #[error("unexpected output of `{command}`: {message}")]
    CommandOutput { command: String, message: String },

    #[error("parse {what}: {source}")]
    ParseInt {
        what: String,
        #[source]
        source: std::num::ParseIntError,
    },

    #[error("parse json {path}: {source}")]
    Json {
        path: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("metadata at {path} is not a JSON object")]
    NotJsonObject { path: String },

    /// Failure of an actual xochitl service control command (`systemctl stop` /
    /// `start xochitl`). Used only by [`crate::tablet::stop_xochitl`] /
    /// [`crate::tablet::start_xochitl`].
    #[error("xochitl: {0}")]
    Xochitl(String),

    /// A read-only [`crate::connection::TabletConnection`] (the CLI's
    /// `--from-backup` shim) was asked to perform a write/exec.
    #[error("backup connection is read-only ({op})")]
    BackupReadOnly { op: &'static str },

    /// Test-only: [`crate::connection::FakeConnection::execute`] received a
    /// command no test fixture had registered output for. Always present in
    /// the variant set so consumer code can match exhaustively, but only
    /// constructed under `#[cfg(feature = "test-utils")]`.
    #[error("fake execute: no registered output for command `{command}`")]
    Mock { command: String },

    #[error(transparent)]
    Metadata(#[from] remarkable_metadata::MetadataError),
}
