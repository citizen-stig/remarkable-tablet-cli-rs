use crate::cli::{GlobalOptions, MkdirArgs};
use crate::error::Result;
use crate::output;

/// # Errors
/// Currently a stub; never errors. Will return SSH/SFTP errors once implemented.
pub fn execute(global: &GlobalOptions, _args: &MkdirArgs) -> Result<()> {
    output::print_not_implemented("mkdir", global.format);
    Ok(())
}
