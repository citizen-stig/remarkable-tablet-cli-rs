use serde_json::json;

pub type Result<T> = std::result::Result<T, CliError>;

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
