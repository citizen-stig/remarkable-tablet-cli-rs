use crate::cli::{GlobalOptions, UploadArgs};
use crate::error::Result;
use crate::output;

/// # Errors
/// Currently a stub; never errors. Will return SSH/SFTP errors once implemented.
pub fn execute(global: &GlobalOptions, _args: &UploadArgs) -> Result<()> {
    output::print_not_implemented("upload", global.format);
    Ok(())
}
