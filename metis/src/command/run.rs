use anyhow::Result;

use crate::command::exec;

#[allow(unused_imports)]
pub use crate::command::exec::eval_with_closure_unwrapping;

pub fn run(script_input: String) -> Result<()> {
    exec::run_script(script_input, None)
}
