use anyhow::{anyhow, Context, Result};
use metis_common::users::{ResolveUserRequest, Username};
use std::{
    env, fs,
    future::Future,
    io::ErrorKind,
    path::{Path, PathBuf},
    pin::Pin,
};

use crate::config;
use crate::{client::MetisClientInterface, command::login};

const AUTH_TOKEN_PATH: &str = "~/.local/share/metis/auth-token";

pub(crate) fn resolve_auth_token_path() -> Result<PathBuf> {
    let home = env::var_os("HOME")
        .ok_or_else(|| anyhow!("HOME is not set; cannot resolve auth token path"))?;
    let raw_path = Path::new(AUTH_TOKEN_PATH);
    let expanded = config::expand_path(raw_path);
    if expanded.to_string_lossy().starts_with('~') {
        return Ok(PathBuf::from(home).join(".local/share/metis/auth-token"));
    }
    Ok(expanded)
}

enum AuthTokenState {
    Missing(PathBuf),
    Empty(PathBuf),
    Present(String),
}

fn read_auth_token_state() -> Result<AuthTokenState> {
    let path = resolve_auth_token_path()?;
    let token = match fs::read_to_string(&path) {
        Ok(token) => token,
        Err(err) => {
            if err.kind() == ErrorKind::NotFound {
                return Ok(AuthTokenState::Missing(path));
            }
            return Err(anyhow!(
                "failed to read auth token from {}: {err}",
                path.display()
            ));
        }
    };

    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Ok(AuthTokenState::Empty(path));
    }

    Ok(AuthTokenState::Present(trimmed.to_string()))
}

#[allow(dead_code)]
pub(crate) fn read_auth_token() -> Result<String> {
    match read_auth_token_state()? {
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

pub(crate) async fn ensure_auth_token(client: &dyn MetisClientInterface) -> Result<String> {
    ensure_auth_token_with_login(client, |client| Box::pin(login::run(client))).await
}

async fn ensure_auth_token_with_login<F>(
    client: &dyn MetisClientInterface,
    login_runner: F,
) -> Result<String>
where
    F: for<'a> Fn(&'a dyn MetisClientInterface) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>>,
{
    match read_auth_token_state()? {
        AuthTokenState::Present(token) => Ok(token),
        AuthTokenState::Missing(_) | AuthTokenState::Empty(_) => {
            login_runner(client)
                .await
                .context("failed to run login flow")?;
            match read_auth_token_state()? {
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
pub(crate) async fn resolve_auth_user(client: &dyn MetisClientInterface) -> Result<Username> {
    let token = ensure_auth_token(client).await?;
    let response = client
        .resolve_user(&ResolveUserRequest::new(token))
        .await
        .context("failed to resolve user from auth token")?;
    Ok(response.user.username)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MetisClient;
    use std::env;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    };
    use tempfile::tempdir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_temp_home<F, R>(action: F) -> R
    where
        F: FnOnce() -> R,
    {
        let _guard = ENV_LOCK.lock().unwrap();
        let original = env::var_os("HOME");
        let temp = tempdir().expect("tempdir");
        env::set_var("HOME", temp.path());

        let result = action();

        match original {
            Some(value) => env::set_var("HOME", value),
            None => env::remove_var("HOME"),
        }

        result
    }

    #[test]
    fn ensure_auth_token_triggers_login_when_missing() {
        with_temp_home(|| {
            let login_calls = AtomicUsize::new(0);
            let client = MetisClient::new("http://localhost").expect("client");
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");

            let token = runtime
                .block_on(ensure_auth_token_with_login(&client, |_client| {
                    login_calls.fetch_add(1, Ordering::SeqCst);
                    let path = resolve_auth_token_path().expect("auth path");
                    fs::create_dir_all(path.parent().expect("auth parent"))
                        .expect("create auth dir");
                    fs::write(&path, "token-abc").expect("write auth token");
                    Box::pin(async { Ok(()) })
                }))
                .expect("ensure auth token");

            assert_eq!(token, "token-abc");
            assert_eq!(login_calls.load(Ordering::SeqCst), 1);
        });
    }

    #[test]
    fn ensure_auth_token_triggers_login_when_empty() {
        with_temp_home(|| {
            let login_calls = AtomicUsize::new(0);
            let client = MetisClient::new("http://localhost").expect("client");
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");
            let path = resolve_auth_token_path().expect("auth path");
            fs::create_dir_all(path.parent().expect("auth parent")).expect("create auth dir");
            fs::write(&path, "   \n").expect("write auth token");

            let token = runtime
                .block_on(ensure_auth_token_with_login(&client, |_client| {
                    login_calls.fetch_add(1, Ordering::SeqCst);
                    let path = resolve_auth_token_path().expect("auth path");
                    fs::write(&path, "token-empty").expect("write auth token");
                    Box::pin(async { Ok(()) })
                }))
                .expect("ensure auth token");

            assert_eq!(token, "token-empty");
            assert_eq!(login_calls.load(Ordering::SeqCst), 1);
        });
    }

    #[test]
    fn ensure_auth_token_skips_login_when_present() {
        with_temp_home(|| {
            let login_calls = AtomicUsize::new(0);
            let client = MetisClient::new("http://localhost").expect("client");
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");
            let path = resolve_auth_token_path().expect("auth path");
            fs::create_dir_all(path.parent().expect("auth parent")).expect("create auth dir");
            fs::write(&path, "  token-123 \n").expect("write auth token");

            let token = runtime
                .block_on(ensure_auth_token_with_login(&client, |_client| {
                    login_calls.fetch_add(1, Ordering::SeqCst);
                    Box::pin(async { Ok(()) })
                }))
                .expect("ensure auth token");

            assert_eq!(token, "token-123");
            assert_eq!(login_calls.load(Ordering::SeqCst), 0);
        });
    }
}
