use crate::cli::RmArgs;
use crate::commands::common::CommandContext;
use crate::error::CliError;

/// # Errors
/// Returns `CliError::NotImplemented` until the command is implemented.
pub async fn execute(_ctx: &CommandContext, _args: &RmArgs) -> Result<(), CliError> {
    Err(CliError::NotImplemented(
        "rm command is not implemented yet".to_string(),
    ))
}
