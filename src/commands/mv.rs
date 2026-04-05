use crate::cli::{GlobalOptions, MvArgs};
use crate::error::Result;
use crate::output;

pub fn execute(global: &GlobalOptions, _args: &MvArgs) -> Result<()> {
    output::print_not_implemented("mv", global.format);
    Ok(())
}
