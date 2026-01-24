use anyhow::{anyhow, Context, Result};
use metis_common::users::{ResolveUserRequest, Username};
use std::{fs, future::Future, io::ErrorKind, path::PathBuf, pin::Pin};

use crate::{
    client::{MetisClientInterface, MetisClientUnauthenticated},
    github_device_flow,
};

pub const DEFAULT_AUTH_TOKEN_PATH: &str = "~/.local/share/metis/auth-token";

enum AuthTokenState {
    Missing(PathBuf),
    Empty(PathBuf),
    Present(String),
}

fn read_auth_token_state(token_path: &PathBuf) -> Result<AuthTokenState> {
    let token = match fs::read_to_string(token_path) {
        Ok(token) => token,
        Err(err) => {
            if err.kind() == ErrorKind::NotFound {
                return Ok(AuthTokenState::Missing(token_path.clone()));
            }
            return Err(anyhow!(
                "failed to read auth token from {}: {err}",
                token_path.display()
            ));
        }
    };

    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Ok(AuthTokenState::Empty(token_path.clone()));
    }

    Ok(AuthTokenState::Present(trimmed.to_string()))
}

#[allow(dead_code)]
pub(crate) fn read_auth_token(token_path: &PathBuf) -> Result<String> {
    match read_auth_token_state(token_path)? {
        AuthTokenState::Missing(path) => Err(anyhow!(
            "Auth token not found at {}. Run `metis login`.",
            path.display()
        )),
        AuthTokenState::Empty(path) => Err(anyhow!(
            "Auth token file at {} is empty. Run `metis login`.",
            path.display()
        )),
        AuthTokenState::Present(token) => Ok(token),
    }
}

pub(crate) async fn ensure_auth_token(
    client: &dyn MetisClientInterface,
    token_path: &PathBuf,
) -> Result<String> {
    let unauth_client = MetisClientUnauthenticated::new(client.base_url().as_str())?;
    ensure_auth_token_with_login(&unauth_client, token_path, |client, token_path| {
        Box::pin(async move {
            let _ = github_device_flow::login_with_github_device_flow(client, token_path).await?;
            Ok(())
        })
    })
    .await
}

async fn ensure_auth_token_with_login<F>(
    client: &MetisClientUnauthenticated,
    token_path: &PathBuf,
    login_runner: F,
) -> Result<String>
where
    F: for<'a> Fn(
        &'a MetisClientUnauthenticated,
        &'a PathBuf,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>>,
{
    match read_auth_token_state(token_path)? {
        AuthTokenState::Present(token) => Ok(token),
        AuthTokenState::Missing(_) | AuthTokenState::Empty(_) => {
            login_runner(client, token_path)
                .await
                .context("failed to run login flow")?;
            match read_auth_token_state(token_path)? {
                AuthTokenState::Present(token) => Ok(token),
                AuthTokenState::Missing(path) => Err(anyhow!(
                    "Auth token not found at {} after login.",
                    path.display()
                )),
                AuthTokenState::Empty(path) => Err(anyhow!(
                    "Auth token file at {} is empty after login.",
                    path.display()
                )),
            }
        }
    }
}

#[allow(dead_code)]
pub(crate) async fn resolve_auth_user(
    client: &dyn MetisClientInterface,
    token_path: &PathBuf,
) -> Result<Username> {
    let token = ensure_auth_token(client, token_path).await?;
    let response = client
        .resolve_user(&ResolveUserRequest::new(token))
        .await
        .context("failed to resolve user from auth token")?;
    Ok(response.user.username)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MetisClientUnauthenticated;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::tempdir;

    #[test]
    fn ensure_auth_token_triggers_login_when_missing() {
        let temp = tempdir().expect("tempdir");
        let token_path = temp.path().join("auth-token");
        let login_calls = AtomicUsize::new(0);
        let client = MetisClientUnauthenticated::new("http://localhost").expect("client");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");

        let token = runtime
            .block_on(ensure_auth_token_with_login(
                &client,
                &token_path,
                |_client, token_path| {
                    login_calls.fetch_add(1, Ordering::SeqCst);
                    fs::create_dir_all(token_path.parent().expect("auth parent"))
                        .expect("create auth dir");
                    fs::write(token_path, "token-abc").expect("write auth token");
                    Box::pin(async { Ok(()) })
                },
            ))
            .expect("ensure auth token");

        assert_eq!(token, "token-abc");
        assert_eq!(login_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn ensure_auth_token_triggers_login_when_empty() {
        let temp = tempdir().expect("tempdir");
        let token_path = temp.path().join("auth-token");
        fs::create_dir_all(token_path.parent().expect("auth parent")).expect("create auth dir");
        fs::write(&token_path, "   \n").expect("write auth token");

        let login_calls = AtomicUsize::new(0);
        let client = MetisClientUnauthenticated::new("http://localhost").expect("client");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");

        let token = runtime
            .block_on(ensure_auth_token_with_login(
                &client,
                &token_path,
                |_client, token_path| {
                    login_calls.fetch_add(1, Ordering::SeqCst);
                    fs::write(token_path, "token-empty").expect("write auth token");
                    Box::pin(async { Ok(()) })
                },
            ))
            .expect("ensure auth token");

        assert_eq!(token, "token-empty");
        assert_eq!(login_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn ensure_auth_token_skips_login_when_present() {
        let temp = tempdir().expect("tempdir");
        let token_path = temp.path().join("auth-token");
        fs::create_dir_all(token_path.parent().expect("auth parent")).expect("create auth dir");
        fs::write(&token_path, "  token-123 \n").expect("write auth token");

        let login_calls = AtomicUsize::new(0);
        let client = MetisClientUnauthenticated::new("http://localhost").expect("client");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");

        let token = runtime
            .block_on(ensure_auth_token_with_login(
                &client,
                &token_path,
                |_client, _token_path| {
                    login_calls.fetch_add(1, Ordering::SeqCst);
                    Box::pin(async { Ok(()) })
                },
            ))
            .expect("ensure auth token");

        assert_eq!(token, "token-123");
        assert_eq!(login_calls.load(Ordering::SeqCst), 0);
    }
}
