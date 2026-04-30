/// Errors produced by the metadata layer (path resolution, tree traversal,
/// `.metadata` / `.content` JSON parsing).
///
/// The CLI wraps these in its own structured `CliError` so JSON consumers see
/// the canonical error codes; library callers can match on these directly.
#[derive(Debug, thiserror::Error)]
pub enum MetadataError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Invalid path: {0}")]
    InvalidPath(String),

    #[error("cycle detected in folder hierarchy at UUID {uuid}")]
    Cycle { uuid: uuid::Uuid },

    #[error("Document {uuid} is missing a valid .content file")]
    MissingContent { uuid: uuid::Uuid },

    #[error("parse {what} JSON: {source}")]
    Parse {
        /// `"metadata"` or `"content"` — names which JSON document failed.
        what: &'static str,
        #[source]
        source: serde_json::Error,
    },
}
