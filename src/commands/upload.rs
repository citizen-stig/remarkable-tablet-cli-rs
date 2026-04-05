use crate::cli::{GlobalOptions, UploadArgs};
use crate::error::Result;
use crate::output;

pub fn execute(global: &GlobalOptions, _args: &UploadArgs) -> Result<()> {
    output::print_not_implemented("upload", global.format);
    Ok(())
}
