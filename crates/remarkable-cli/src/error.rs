use serde_json::json;

use remarkable_metadata::MetadataError;
use remarkable_tablet::TabletError;

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Authentication failed: {0}")]
    AuthFailed(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Already exists: {0}")]
    AlreadyExists(String),

    #[error("Invalid path: {0}")]
    InvalidPath(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Xochitl error: {0}")]
    XochitlError(String),

    #[error("Format error: {0}")]
    FormatError(String),

    #[error("Not implemented: {0}")]
    NotImplemented(String),

    #[error("IO error: {0}")]
    IoError(String),
}

impl CliError {
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::ConnectionFailed(_) => "connection_failed",
            Self::AuthFailed(_) => "auth_failed",
            Self::NotFound(_) => "not_found",
            Self::AlreadyExists(_) => "already_exists",
            Self::InvalidPath(_) => "invalid_path",
            Self::PermissionDenied(_) => "permission_denied",
            Self::XochitlError(_) => "xochitl_error",
            Self::FormatError(_) => "format_error",
            Self::NotImplemented(_) => "not_implemented",
            Self::IoError(_) => "io_error",
        }
    }

    #[must_use]
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::ConnectionFailed(_) => 1,
            Self::AuthFailed(_) => 2,
            Self::NotFound(_) => 3,
            Self::AlreadyExists(_) => 4,
            Self::InvalidPath(_) => 5,
            Self::PermissionDenied(_) => 6,
            Self::XochitlError(_) => 7,
            Self::FormatError(_) => 8,
            Self::IoError(_) => 9,
            Self::NotImplemented(_) => 10,
        }
    }

    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "error": true,
            "code": self.code(),
            "message": self.to_string(),
        })
    }
}

impl From<std::io::Error> for CliError {
    fn from(err: std::io::Error) -> Self {
        Self::IoError(err.to_string())
    }
}

impl From<MetadataError> for CliError {
    fn from(err: MetadataError) -> Self {
        match err {
            MetadataError::NotFound(msg) => Self::NotFound(msg),
            MetadataError::InvalidPath(msg) => Self::InvalidPath(msg),
            // Tablet-data-shape failures (cycles, missing .content, JSON parse)
            // are surfaced as IoError so the CLI keeps a single bucket for
            // "the tablet's data is structurally wrong"; the Display impl on
            // each MetadataError variant already includes the source chain.
            other @ (MetadataError::Cycle { .. }
            | MetadataError::MissingContent { .. }
            | MetadataError::Parse { .. }) => Self::IoError(other.to_string()),
        }
    }
}

impl From<TabletError> for CliError {
    fn from(err: TabletError) -> Self {
        match err {
            TabletError::ConnectTimeout { .. } | TabletError::Connect { .. } => {
                Self::ConnectionFailed(err.to_string())
            }
            TabletError::AuthFailed(msg) => Self::AuthFailed(msg),
            TabletError::Xochitl(msg) => Self::XochitlError(msg),
            TabletError::BackupReadOnly { .. } => Self::NotImplemented(err.to_string()),
            // Pass nested MetadataError through its own From impl.
            TabletError::Metadata(meta) => meta.into(),
            // Everything else (filesystem I/O, ssh, JSON, parse, command
            // shape, utf-8, mock setup) is "the tablet's data or transport
            // went sideways" — bucket into IoError so the JSON envelope is
            // consistent.
            TabletError::Io { .. }
            | TabletError::Ssh { .. }
            | TabletError::CommandOutputNotUtf8 { .. }
            | TabletError::CommandOutput { .. }
            | TabletError::ParseInt { .. }
            | TabletError::Json { .. }
            | TabletError::NotJsonObject { .. }
            | TabletError::Mock { .. } => Self::IoError(err.to_string()),
        }
    }
}
