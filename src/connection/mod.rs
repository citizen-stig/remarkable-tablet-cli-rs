use std::future::Future;
use std::path::Path;

pub mod fake;
pub mod ssh;

pub use fake::FakeConnection;
pub use ssh::{ConnectOptions, SshConnection};

pub trait TabletConnection: Send + Sync {
    fn read_file<P: AsRef<Path> + Send>(
        &self,
        path: P,
    ) -> impl Future<Output = anyhow::Result<Vec<u8>>> + Send;
    // TODO: for larger files we might prefer a buffer or stream; revisit when
    // upload/download commands land.
    fn write_file<P: AsRef<Path> + Send>(
        &self,
        path: P,
        data: &[u8],
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn list_dir<P: AsRef<Path> + Send>(
        &self,
        path: P,
    ) -> impl Future<Output = anyhow::Result<Vec<String>>> + Send;
    fn remove_file<P: AsRef<Path> + Send>(
        &self,
        path: P,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn execute(&self, command: &str) -> impl Future<Output = anyhow::Result<String>> + Send;
    fn file_exists<P: AsRef<Path> + Send>(
        &self,
        path: P,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send;
}
