mod codex;
mod github;
mod shell;

use std::collections::HashMap;

use anyhow::{anyhow, Result};

use self::{
    codex::{codex, evaluate_codex_op},
    github::{
        create_pull_request, evaluate_create_pull_request, evaluate_wait_for_pull_request,
        wait_for_pull_request,
    },
    shell::{evaluate_shell_command, shell},
};

#[derive(Debug, Clone)]
enum AsyncOp {
    Codex {
        prompt: String,
    },
    Shell {
        command: String,
    },
    GithubCreatePullRequest {
        owner: String,
        repo: String,
        title: String,
        head: String,
        base: String,
        body: Option<String>,
    },
    GithubWaitForPullRequest {
        owner: String,
        repo: String,
        number: i64,
        poll_interval_secs: u64,
    },
}

impl std::fmt::Display for AsyncOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AsyncOp::Codex { prompt } => write!(f, "Codex {{ prompt: {prompt} }}"),
            AsyncOp::Shell { command } => write!(f, "Shell {{ command: {command} }}"),
            AsyncOp::GithubCreatePullRequest {
                owner,
                repo,
                title,
                head,
                base,
                body,
            } => write!(
                f,
                "GithubCreatePullRequest {{ owner: {owner}, repo: {repo}, title: {title}, head: {head}, base: {base}, body: {body:?} }}"
            ),
            AsyncOp::GithubWaitForPullRequest {
                owner,
                repo,
                number,
                poll_interval_secs,
            } => write!(
                f,
                "GithubWaitForPullRequest {{ owner: {owner}, repo: {repo}, number: {number}, poll_interval_secs: {poll_interval_secs} }}"
            ),
        }
    }
}

async fn evaluate_async_op(op: &AsyncOp, env: &HashMap<String, String>) -> Result<String> {
    match op {
        AsyncOp::Codex { prompt } => evaluate_codex_op(prompt).await,
        AsyncOp::Shell { command } => evaluate_shell_command(command, env).await,
        AsyncOp::GithubCreatePullRequest {
            owner,
            repo,
            title,
            head,
            base,
            body,
        } => evaluate_create_pull_request(owner, repo, title, head, base, body, env).await,
        AsyncOp::GithubWaitForPullRequest {
            owner,
            repo,
            number,
            poll_interval_secs,
        } => evaluate_wait_for_pull_request(owner, repo, *number, *poll_interval_secs, env).await,
    }
}

/// Evaluates a script and recursively evaluates async operation continuations until the result is no longer a tuple of operation + closure.
pub async fn eval_with_closure_unwrapping(
    script: &str,
    params: Vec<String>,
    env: &HashMap<String, String>,
) -> Result<rhai::Dynamic> {
    let mut engine = rhai::Engine::new();
    // Configure engine limits to support complex scripts with nested closures and function calls
    engine.set_max_expr_depths(256, 256);
    engine.set_max_call_levels(128);
    engine.set_max_operations(50_000);
    engine.register_type_with_name::<AsyncOp>("AsyncOp");
    engine.register_fn("codex", codex);
    engine.register_fn("github_create_pull_request", create_pull_request);
    engine.register_fn("github_wait_for_pull_request", wait_for_pull_request);
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
            let op_result = evaluate_async_op(&op, env).await?;
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

    #[tokio::test]
    async fn eval_with_closure_unwrapping_pushes_env_map() -> Result<()> {
        let mut env = HashMap::new();
        env.insert("API_KEY".to_string(), "secret".to_string());

        let result = eval_with_closure_unwrapping(r#"env["API_KEY"]"#, Vec::new(), &env).await?;
        assert_eq!(
            result
                .try_cast::<String>()
                .expect("Rhai result should be a string"),
            "secret"
        );

        Ok(())
    }

    #[tokio::test]
    async fn shell_commands_use_provided_env() -> Result<()> {
        let mut env = HashMap::new();
        env.insert("GREETING".to_string(), "hello".to_string());

        let result = eval_with_closure_unwrapping(
            r#"shell("printf %s \"$GREETING\"", |output| output)"#,
            Vec::new(),
            &env,
        )
        .await?;

        assert_eq!(
            result
                .try_cast::<String>()
                .expect("Rhai result should be a string"),
            "hello"
        );

        Ok(())
    }
}
