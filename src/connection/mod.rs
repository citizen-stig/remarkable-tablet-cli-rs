use std::time::SystemTime;

pub mod fake;
pub mod ssh;

pub use fake::FakeConnection;
pub use ssh::{ConnectOptions, SshConnection};

/// Kind of a remote filesystem entry. Symlinks and other oddities are
/// collapsed into [`RemoteFileKind::Other`] — the walker treats them as
/// leaves and the backup code skips them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteFileKind {
    File,
    Dir,
    Other,
}

/// Metadata about a single remote filesystem entry.
///
/// Sizes and mtimes come from SFTP attributes when available; remote servers
/// may legitimately omit them (the protocol marks every attribute optional),
/// so callers must tolerate `None`.
#[derive(Debug, Clone)]
pub struct RemoteMetadata {
    pub size: Option<u64>,
    pub mtime: Option<SystemTime>,
    pub kind: RemoteFileKind,
}

/// One entry returned from [`TabletConnection::read_dir`]. Carries the
/// basename (no leading directory) plus its metadata so callers don't need
/// a follow-up `stat` round-trip per file.
#[derive(Debug, Clone)]
pub struct RemoteEntry {
    pub name: String,
    pub metadata: RemoteMetadata,
}

/// Abstraction over a connection to the tablet's filesystem.
///
/// Paths are `&str` rather than `std::path::Path` because they represent remote
/// SFTP paths, which are UTF-8 strings per RFC 4251 §5 and
/// draft-ietf-secsh-filexfer. `Path` carries OS-native semantics (arbitrary
/// bytes on Unix, UCS-2 on Windows) that don't apply to remote paths.
#[allow(async_fn_in_trait)] // crate-internal trait; Send bounds not needed
pub trait TabletConnection {
    async fn read_file(&self, path: &str) -> anyhow::Result<Vec<u8>>;
    // TODO: for larger files we might prefer a buffer or stream; revisit when
    // upload/download commands grow streaming requirements.
    async fn write_file(&self, path: &str, data: &[u8]) -> anyhow::Result<()>;
    async fn read_dir(&self, path: &str) -> anyhow::Result<Vec<RemoteEntry>>;
    async fn stat(&self, path: &str) -> anyhow::Result<RemoteMetadata>;
    async fn remove_file(&self, path: &str) -> anyhow::Result<()>;
    /// Recursively remove a directory and everything under it. No-op if the
    /// path doesn't exist (matches `rm -rf`'s contract). Used by `rm
    /// --permanent` to wipe per-notebook page directories and thumbnail
    /// directories that accumulate under a UUID.
    async fn remove_dir_all(&self, path: &str) -> anyhow::Result<()>;
    async fn execute(&self, command: &str) -> anyhow::Result<String>;
    async fn file_exists(&self, path: &str) -> anyhow::Result<bool>;
}
