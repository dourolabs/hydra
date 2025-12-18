use anyhow::{Context, Result};
use std::{fs, path::Path};

/// Run a Rhai script, reading it from a file when the input points to an existing path.
/// If `base_dir` is provided, relative paths are resolved against it.
pub fn run_script(script_input: String, base_dir: Option<&Path>) -> Result<()> {
    let script = load_script(&script_input, base_dir)?;

    let _ = eval_with_closure_unwrapping(&script)
        .map_err(|err| anyhow::anyhow!("failed to execute Rhai script: {}", err))?;

    Ok(())
}

fn load_script(script_input: &str, base_dir: Option<&Path>) -> Result<String> {
    let path = Path::new(script_input);
    if path.exists() {
        return fs::read_to_string(path)
            .with_context(|| format!("failed to read script file '{}'", script_input));
    }

    if let Some(base_dir) = base_dir {
        let candidate = base_dir.join(script_input);
        if candidate.exists() {
            return fs::read_to_string(&candidate)
                .with_context(|| format!("failed to read script file '{:?}'", candidate));
        }
    }

    Ok(script_input.to_string())
}

/// Evaluates a script and recursively evaluates no-argument closures until the result is no longer a closure.
pub fn eval_with_closure_unwrapping(script: &str) -> Result<rhai::Dynamic> {
    let engine = rhai::Engine::new();
    let ast = engine
        .compile(script)
        .map_err(|err| anyhow::anyhow!("failed to compile Rhai script: {}", err))?;

    let mut scope = rhai::Scope::new();
    let mut result = engine
        .eval_ast_with_scope::<rhai::Dynamic>(&mut scope, &ast)
        .map_err(|err| anyhow::anyhow!("failed to evaluate Rhai script: {}", err))?;

    // Recursively evaluate closures with no arguments
    loop {
        // Check if the result is a closure (FnPtr)
        if let Some(fn_ptr) = result.clone().try_cast::<rhai::FnPtr>() {
            println!("Evaluating closure");
            // Try to call the closure with no arguments
            // If it succeeds, it's a no-argument closure; if it fails, it requires arguments
            match fn_ptr.call(&engine, &ast, ()) {
                Ok(new_result) => {
                    println!("Result: {:?}", &new_result);
                    // Successfully called with no arguments, continue recursion
                    result = new_result;
                    continue;
                }
                Err(err) => {
                    println!("Error: {:?}", &err);
                    // Failed to call - either requires arguments or is not callable
                    // Break the loop and return the closure as-is
                    break;
                }
            }
        } else {
            println!("Not a closure -- done!. Result {:?}", result);
            // Not a closure, return the result
            break;
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn run_script_reads_from_base_dir() -> Result<()> {
        let tempdir = tempdir().context("failed to create tempdir for script test")?;
        let script_path = tempdir.path().join("script.rhai");
        fs::write(&script_path, "1 + 1").context("failed to write script file")?;

        run_script("script.rhai".to_string(), Some(tempdir.path()))
            .context("expected script to be loaded from base dir")?;

        Ok(())
    }

    #[test]
    fn eval_with_closure_unwrapping_executes_no_arg_closure() -> Result<()> {
        let result = eval_with_closure_unwrapping("|| 21 + 21")?;
        let value = result
            .as_int()
            .map_err(|err| anyhow::anyhow!("failed to read int result: {}", err))?;
        assert_eq!(value, 42);
        Ok(())
    }
}
