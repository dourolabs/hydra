use anyhow::{anyhow, Context, Result};
use metis_common::users::{ResolveUserRequest, Username};
use std::{
    env, fs,
    io::ErrorKind,
    path::{Path, PathBuf},
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
    match read_auth_token_state()? {
        AuthTokenState::Present(token) => Ok(token),
        AuthTokenState::Missing(_) | AuthTokenState::Empty(_) => {
            login::run(client)
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
