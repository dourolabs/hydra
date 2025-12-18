use std::{collections::HashMap, fs, path::Path, process::Command};

use anyhow::{anyhow, Context, Result};

#[derive(Debug, Clone)]
enum AsyncOp {
    Codex { prompt: String },
    Shell { command: String },
}

impl std::fmt::Display for AsyncOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AsyncOp::Codex { prompt } => write!(f, "Codex {{ prompt: {prompt} }}"),
            AsyncOp::Shell { command } => write!(f, "Shell {{ command: {command} }}"),
        }
    }
}

fn codex(prompt: String, continuation: rhai::FnPtr) -> (AsyncOp, rhai::FnPtr) {
    (AsyncOp::Codex { prompt }, continuation)
}

fn shell(command: String, continuation: rhai::FnPtr) -> (AsyncOp, rhai::FnPtr) {
    (AsyncOp::Shell { command }, continuation)
}

fn evaluate_async_op(op: &AsyncOp, env: &HashMap<String, String>) -> Result<String> {
    match op {
        AsyncOp::Codex { prompt } => evaluate_codex_op(prompt),
        AsyncOp::Shell { command } => evaluate_shell_command(command, env),
    }
}

fn evaluate_codex_op(prompt: &str) -> Result<String> {
    let output_path = Path::new(".metis/output/output.txt");
    if let Some(dir) = output_path.parent() {
        fs::create_dir_all(dir)
            .with_context(|| format!("failed to create codex output directory {dir:?}"))?;
    }

    let status = Command::new("codex")
        .args([
            "exec",
            "-o",
            output_path
                .to_str()
                .expect("codex output path should be valid UTF-8"),
            "--dangerously-bypass-approvals-and-sandbox",
            prompt,
        ])
        .status()
        .context("failed to spawn codex command")?;

    if !status.success() {
        return Err(anyhow!("codex command failed with status {status}"));
    }

    fs::read_to_string(output_path)
        .with_context(|| format!("failed to read codex output from {output_path:?}"))
}

fn evaluate_shell_command(command: &str, env: &HashMap<String, String>) -> Result<String> {
    let output = Command::new("sh")
        .args(["-c", command])
        .envs(env)
        .output()
        .with_context(|| format!("failed to spawn shell command: {command}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "shell command `{command}` failed with status {}{}",
            output.status,
            if stderr.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", stderr.trim())
            }
        ));
    }

    String::from_utf8(output.stdout)
        .map_err(|err| anyhow!("failed to decode shell command output as UTF-8: {err}"))
}

/// Evaluates a script and recursively evaluates async operation continuations until the result is no longer a tuple of operation + closure.
pub fn eval_with_closure_unwrapping(
    script: &str,
    params: Vec<String>,
    env: &HashMap<String, String>,
) -> Result<rhai::Dynamic> {
    let mut engine = rhai::Engine::new();
    engine.register_type_with_name::<AsyncOp>("AsyncOp");
    engine.register_fn("codex", codex);
    engine.register_fn("shell", shell);

    let ast = engine
        .compile(script)
        .map_err(|err| anyhow!("failed to compile Rhai script: {}", err))?;

    let mut scope = rhai::Scope::new();
    let params_array: rhai::Array = params.into_iter().map(|value| value.into()).collect();
    scope.push("params", params_array);
    let mut env_map = rhai::Map::new();
    for (key, value) in env {
        env_map.insert(key.into(), value.clone().into());
    }
    scope.push("env", env_map);
    let mut result = engine
        .eval_ast_with_scope::<rhai::Dynamic>(&mut scope, &ast)
        .map_err(|err| anyhow!("failed to evaluate Rhai script: {}", err))?;

    // Recursively evaluate async operations by executing their continuations
    loop {
        if let Some((op, fn_ptr)) = result.clone().try_cast::<(AsyncOp, rhai::FnPtr)>() {
            println!("Async op: {}", op);
            let op_result = evaluate_async_op(&op, env)?;
            match fn_ptr.call(&engine, &ast, (op_result,)) {
                Ok(new_result) => {
                    println!("Result: {:?}", &new_result);
                    // Successfully called with async op output, continue recursion
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_with_closure_unwrapping_pushes_env_map() -> Result<()> {
        let mut env = HashMap::new();
        env.insert("API_KEY".to_string(), "secret".to_string());

        let result = eval_with_closure_unwrapping(r#"env["API_KEY"]"#, Vec::new(), &env)?;
        assert_eq!(
            result
                .try_cast::<String>()
                .expect("Rhai result should be a string"),
            "secret"
        );

        Ok(())
    }

    #[test]
    fn shell_commands_use_provided_env() -> Result<()> {
        let mut env = HashMap::new();
        env.insert("GREETING".to_string(), "hello".to_string());

        let result = eval_with_closure_unwrapping(
            r#"shell("printf %s \"$GREETING\"", |output| output)"#,
            Vec::new(),
            &env,
        )?;

        assert_eq!(
            result
                .try_cast::<String>()
                .expect("Rhai result should be a string"),
            "hello"
        );

        Ok(())
    }
}
