use crate::cli::MvArgs;
use crate::commands::common::CommandContext;
use crate::error::CliError;

/// # Errors
/// Returns `CliError::NotImplemented` until the command is implemented.
pub async fn execute(_ctx: &CommandContext, _args: &MvArgs) -> Result<(), CliError> {
    Err(CliError::NotImplemented(
        "mv command is not implemented yet".to_string(),
    ))
}
