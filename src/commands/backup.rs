use crate::cli::BackupArgs;
use crate::commands::common::CommandContext;
use crate::error::CliError;

/// # Errors
/// Returns `CliError::NotImplemented` until the command is implemented.
pub async fn execute(_ctx: &CommandContext, _args: &BackupArgs) -> Result<(), CliError> {
    Err(CliError::NotImplemented(
        "backup command is not implemented yet".to_string(),
    ))
}
