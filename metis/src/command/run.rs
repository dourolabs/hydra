use crate::exec::eval_with_closure_unwrapping;
use anyhow::{Context, Result};
use std::{collections::HashMap, path::Path};

pub fn run(script_input: String) -> Result<()> {
    // Determine if input is a file path or a script string
    let script = if Path::new(&script_input).exists() {
        std::fs::read_to_string(&script_input)
            .with_context(|| format!("failed to read script file '{}'", script_input))?
    } else {
        script_input
    };

    // Run the script
    let _ = eval_with_closure_unwrapping(&script, Vec::new(), &HashMap::new())
        .map_err(|err| anyhow::anyhow!("failed to execute Rhai script: {}", err))?;

    Ok(())
}
