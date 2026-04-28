/// Errors produced by the tablet client (connection, SFTP, xochitl control).
///
/// The CLI wraps these in its own structured `CliError` so JSON consumers see
/// the canonical error codes; library callers can match on these directly.
#[derive(Debug, thiserror::Error)]
pub enum TabletError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Authentication failed: {0}")]
    AuthFailed(String),

    #[error("Xochitl error: {0}")]
    XochitlError(String),
}
