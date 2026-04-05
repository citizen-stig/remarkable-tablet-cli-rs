use crate::cli::{GlobalOptions, LsArgs};
use crate::error::Result;
use crate::output;

pub fn execute(global: &GlobalOptions, _args: &LsArgs) -> Result<()> {
    output::print_not_implemented("ls", global.format);
    Ok(())
}
