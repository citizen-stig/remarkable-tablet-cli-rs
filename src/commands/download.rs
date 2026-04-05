use crate::cli::{GlobalOptions, DownloadArgs};
use crate::error::Result;
use crate::output;

pub fn execute(global: &GlobalOptions, _args: &DownloadArgs) -> Result<()> {
    output::print_not_implemented("download", global.format);
    Ok(())
}
