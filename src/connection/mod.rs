pub mod fake;
pub mod ssh;

pub use fake::FakeConnection;
pub use ssh::{ConnectOptions, SshConnection};

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
    // upload/download commands land.
    async fn write_file(&self, path: &str, data: &[u8]) -> anyhow::Result<()>;
    async fn list_dir(&self, path: &str) -> anyhow::Result<Vec<String>>;
    async fn remove_file(&self, path: &str) -> anyhow::Result<()>;
    async fn execute(&self, command: &str) -> anyhow::Result<String>;
    async fn file_exists(&self, path: &str) -> anyhow::Result<bool>;
}
