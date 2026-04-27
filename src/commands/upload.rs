use crate::cli::UploadArgs;
use crate::commands::common::CommandContext;
use crate::error::CliError;

/// # Errors
/// Returns `CliError::NotImplemented` until the command is implemented.
pub async fn execute(_ctx: &CommandContext, _args: &UploadArgs) -> Result<(), CliError> {
    Err(CliError::NotImplemented(
        "upload command is not implemented yet".to_string(),
    ))
}
