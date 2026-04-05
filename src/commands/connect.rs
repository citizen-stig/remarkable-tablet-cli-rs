use crate::cli::GlobalOptions;
use crate::error::Result;
use crate::output;

pub fn execute(global: &GlobalOptions) -> Result<()> {
    output::print_not_implemented("connect", global.format);
    Ok(())
}
