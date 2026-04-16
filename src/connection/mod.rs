use std::future::Future;

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
pub trait TabletConnection: Send + Sync {
    fn read_file(
        &self,
        path: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<u8>>> + Send;
    // TODO: for larger files we might prefer a buffer or stream; revisit when
    // upload/download commands land.
    fn write_file(
        &self,
        path: &str,
        data: &[u8],
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn list_dir(
        &self,
        path: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<String>>> + Send;
    fn remove_file(
        &self,
        path: &str,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn execute(&self, command: &str) -> impl Future<Output = anyhow::Result<String>> + Send;
    fn file_exists(
        &self,
        path: &str,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send;
}
