//! Shared fixtures for the `remarkable-cli` integration tests.
//!
//! Cargo compiles `tests/common/mod.rs` into each test binary that declares
//! `mod common;` rather than as a test binary of its own. Each suite pulls in
//! a different subset of these helpers, so `dead_code` is allowed here.
#![allow(dead_code)]

use remarkable_cli::error::CliError;
use remarkable_metadata::tree::DocumentTree;
use remarkable_tablet::connection::FakeConnection;
use remarkable_tablet::tablet::load_all_metadata;

/// Remote xochitl data directory the fixtures are laid down under.
pub const DATA_DIR: &str = "/home/root/.local/share/remarkable/xochitl";

/// Load every `.metadata` file from `conn` and assemble the document tree.
pub async fn build_tree(conn: &FakeConnection) -> DocumentTree {
    let entries = load_all_metadata(conn, DATA_DIR).await.unwrap();
    DocumentTree::build(entries)
}

/// Register no-op outputs for the `systemctl stop/start xochitl` calls that
/// mutating commands bracket their writes with.
pub fn register_xochitl(conn: &FakeConnection) {
    conn.set_command_output("systemctl stop xochitl", "");
    conn.set_command_output("systemctl start xochitl", "");
}

/// Downcast the `anyhow::Error` a command returns back into its `CliError`,
/// for asserting on the error variant.
pub fn downcast_cli(err: anyhow::Error) -> CliError {
    err.downcast::<CliError>().expect("expected CliError")
}
