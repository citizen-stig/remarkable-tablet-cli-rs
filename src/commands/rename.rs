use crate::cli::RenameArgs;
use crate::commands::common::CommandContext;
use crate::error::CliError;

/// # Errors
/// Returns `CliError::NotImplemented` until the command is implemented.
pub async fn execute(_ctx: &CommandContext, _args: &RenameArgs) -> Result<(), CliError> {
    Err(CliError::NotImplemented(
        "rename command is not implemented yet".to_string(),
    ))
}
