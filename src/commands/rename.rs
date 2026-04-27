use crate::cli::{GlobalOptions, RenameArgs};
use crate::error::Result;
use crate::output;

/// # Errors
/// Currently a stub; never errors. Will return SSH/SFTP errors once implemented.
pub fn execute(global: &GlobalOptions, _args: &RenameArgs) -> Result<()> {
    output::print_not_implemented("rename", global.format);
    Ok(())
}
