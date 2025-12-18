use anyhow::{anyhow, Result};

/// Evaluates a script and recursively evaluates no-argument closures until the result is no longer a closure.
pub fn eval_with_closure_unwrapping(script: &str) -> Result<rhai::Dynamic> {
    let engine = rhai::Engine::new();
    let ast = engine
        .compile(script)
        .map_err(|err| anyhow!("failed to compile Rhai script: {}", err))?;

    let mut scope = rhai::Scope::new();
    let mut result = engine
        .eval_ast_with_scope::<rhai::Dynamic>(&mut scope, &ast)
        .map_err(|err| anyhow!("failed to evaluate Rhai script: {}", err))?;

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
