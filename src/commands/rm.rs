use crate::cli::{GlobalOptions, RmArgs};
use crate::error::Result;
use crate::output;

/// # Errors
/// Currently a stub; never errors. Will return SSH/SFTP errors once implemented.
pub fn execute(global: &GlobalOptions, _args: &RmArgs) -> Result<()> {
    output::print_not_implemented("rm", global.format);
    Ok(())
}
