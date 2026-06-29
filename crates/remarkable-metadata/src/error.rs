use std::fmt::Write as _;

use serde::Serialize;
use uuid::Uuid;

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

    /// A path segment matched more than one sibling with the same name. Carries
    /// the candidates so callers (and LLM agents) can re-issue the command with
    /// a specific UUID instead of guessing.
    #[error("{}", ambiguous_message(.name, .candidates))]
    Ambiguous {
        name: String,
        candidates: Vec<AmbiguousCandidate>,
    },

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

/// One entry that contributed to an ambiguous path: a sibling sharing the
/// requested name. `kind` is a single-word label (`folder`, `pdf`, `epub`,
/// `notebook`, `template`); `path` is the entry's full display path.
#[derive(Debug, Clone, Serialize)]
pub struct AmbiguousCandidate {
    pub uuid: Uuid,
    pub kind: &'static str,
    pub path: String,
}

/// Render the human-readable body of an [`MetadataError::Ambiguous`]: a one-line
/// summary followed by one `  <uuid>  (<kind>)  <path>` line per candidate.
/// Shared by the error's `Display` impl and the CLI's JSON-error mapping so the
/// prose stays in one place.
#[must_use]
pub fn ambiguous_message(name: &str, candidates: &[AmbiguousCandidate]) -> String {
    let mut s = format!(
        "ambiguous: {} items named '{name}' in the same folder — retry with a UUID:",
        candidates.len()
    );
    for c in candidates {
        let _ = write!(s, "\n  {}  ({})  {}", c.uuid, c.kind, c.path);
    }
    s
}
