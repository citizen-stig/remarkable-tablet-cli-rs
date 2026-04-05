use crate::cli::{GlobalOptions, BackupArgs};
use crate::error::Result;
use crate::output;

pub fn execute(global: &GlobalOptions, _args: &BackupArgs) -> Result<()> {
    output::print_not_implemented("backup", global.format);
    Ok(())
}
