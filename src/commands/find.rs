use crate::cli::{FindArgs, GlobalOptions};
use crate::error::Result;
use crate::output;

pub fn execute(global: &GlobalOptions, _args: &FindArgs) -> Result<()> {
    output::print_not_implemented("find", global.format);
    Ok(())
}
