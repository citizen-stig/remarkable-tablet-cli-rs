/// Errors produced by the metadata layer (path resolution, tree traversal).
///
/// The CLI wraps these in its own structured `CliError` so JSON consumers see
/// the canonical error codes; library callers can match on these directly.
#[derive(Debug, thiserror::Error)]
pub enum MetadataError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Invalid path: {0}")]
    InvalidPath(String),
}
