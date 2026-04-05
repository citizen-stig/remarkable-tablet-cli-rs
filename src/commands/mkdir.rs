use crate::cli::{GlobalOptions, MkdirArgs};
use crate::error::Result;
use crate::output;

pub fn execute(global: &GlobalOptions, _args: &MkdirArgs) -> Result<()> {
    output::print_not_implemented("mkdir", global.format);
    Ok(())
}
