use crate::cli::{BackupArgs, GlobalOptions};
use crate::error::Result;
use crate::output;

/// # Errors
/// Currently a stub; never errors. Will return SSH/SFTP errors once implemented.
pub fn execute(global: &GlobalOptions, _args: &BackupArgs) -> Result<()> {
    output::print_not_implemented("backup", global.format);
    Ok(())
}
