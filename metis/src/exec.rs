use anyhow::{anyhow, Result};

#[derive(Debug, Clone)]
enum AsyncOp {
    Codex { prompt: String },
}

impl std::fmt::Display for AsyncOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AsyncOp::Codex { prompt } => write!(f, "Codex {{ prompt: {prompt} }}"),
        }
    }
}

fn codex(prompt: String, continuation: rhai::FnPtr) -> (AsyncOp, rhai::FnPtr) {
    (AsyncOp::Codex { prompt }, continuation)
}

/// Evaluates a script and recursively evaluates async operation continuations until the result is no longer a tuple of operation + closure.
pub fn eval_with_closure_unwrapping(script: &str) -> Result<rhai::Dynamic> {
    let mut engine = rhai::Engine::new();
    engine.register_type_with_name::<AsyncOp>("AsyncOp");
    engine.register_fn("codex", codex);

    let ast = engine
        .compile(script)
        .map_err(|err| anyhow!("failed to compile Rhai script: {}", err))?;

    let mut scope = rhai::Scope::new();
    let mut result = engine
        .eval_ast_with_scope::<rhai::Dynamic>(&mut scope, &ast)
        .map_err(|err| anyhow!("failed to evaluate Rhai script: {}", err))?;

    // Recursively evaluate async operations by executing their continuations
    loop {
        if let Some((op, fn_ptr)) = result.clone().try_cast::<(AsyncOp, rhai::FnPtr)>() {
            println!("Async op: {}", op);
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
                    // Break the loop and return the continuation as-is
                    break;
                }
            }
        } else {
            println!("Not an async op tuple -- done!. Result {:?}", result);
            // Not an async operation tuple, return the result
            break;
        }
    }

    Ok(result)
}
