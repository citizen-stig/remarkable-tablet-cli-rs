//! SSH/SFTP client and high-level filesystem operations for the reMarkable
//! tablet, layered on top of the data-only `remarkable-metadata` crate.
//!
//! The [`connection::TabletConnection`] trait abstracts the remote filesystem
//! so commands can be exercised against [`connection::FakeConnection`]
//! (gated behind the `test-utils` feature) in offline tests, while production
//! code uses [`connection::SshConnection`].

pub mod connection;
pub mod error;
pub mod metadata_loader;
pub mod tablet;
pub mod transfer;

pub use connection::{
    ConnectOptions, RemoteEntry, RemoteFileKind, RemoteMetadata, SshConnection, TabletConnection,
};
#[cfg(feature = "test-utils")]
pub use connection::FakeConnection;
pub use error::TabletError;
pub use metadata_loader::{LoadDiagnostics, load_all_metadata, load_all_metadata_full};
pub use tablet::{
    ConnectionType, DeviceInfo, fetch_device_info, start_xochitl, stop_xochitl, update_metadata,
    with_xochitl_stopped,
};
pub use transfer::{
    TRANSFER_CONCURRENCY, WalkedFile, download_file, download_many, upload_file, walk_remote,
};
