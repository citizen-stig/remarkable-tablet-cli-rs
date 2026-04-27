use crate::cli::{GlobalOptions, MvArgs};
use crate::error::Result;
use crate::output;

/// # Errors
/// Currently a stub; never errors. Will return SSH/SFTP errors once implemented.
pub fn execute(global: &GlobalOptions, _args: &MvArgs) -> Result<()> {
    output::print_not_implemented("mv", global.format);
    Ok(())
}
