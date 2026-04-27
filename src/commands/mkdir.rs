use crate::cli::MkdirArgs;
use crate::commands::common::CommandContext;
use crate::error::CliError;

/// # Errors
/// Returns `CliError::NotImplemented` until the command is implemented.
pub async fn execute(_ctx: &CommandContext, _args: &MkdirArgs) -> Result<(), CliError> {
    Err(CliError::NotImplemented(
        "mkdir command is not implemented yet".to_string(),
    ))
}
