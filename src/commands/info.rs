use crate::cli::{GlobalOptions, InfoArgs};
use crate::error::Result;
use crate::output;

pub fn execute(global: &GlobalOptions, _args: &InfoArgs) -> Result<()> {
    output::print_not_implemented("info", global.format);
    Ok(())
}
