use crate::cli::DownloadArgs;
use crate::commands::common::CommandContext;
use crate::error::CliError;

/// # Errors
/// Returns `CliError::NotImplemented` until the command is implemented.
pub async fn execute(_ctx: &CommandContext, _args: &DownloadArgs) -> Result<(), CliError> {
    Err(CliError::NotImplemented(
        "download command is not implemented yet".to_string(),
    ))
}
