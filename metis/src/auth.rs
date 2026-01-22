use anyhow::{anyhow, Context, Result};
use metis_common::users::{ResolveUserRequest, Username};
use std::{
    env, fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use crate::client::MetisClientInterface;
use crate::config;

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

pub fn read_auth_token() -> Result<String> {
    let path = resolve_auth_token_path()?;
    let token = fs::read_to_string(&path).map_err(|err| {
        if err.kind() == ErrorKind::NotFound {
            anyhow!(
                "Auth token not found at {}. Run `metis login`.",
                path.display()
            )
        } else {
            anyhow!("failed to read auth token from {}: {err}", path.display())
        }
    })?;

    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Err(anyhow!(
            "Auth token file at {} is empty. Run `metis login`.",
            path.display()
        ));
    }

    Ok(trimmed.to_string())
}

#[allow(dead_code)]
pub(crate) async fn resolve_auth_user(client: &dyn MetisClientInterface) -> Result<Username> {
    let token = read_auth_token()?;
    let response = client
        .resolve_user(&ResolveUserRequest::new(token))
        .await
        .context("failed to resolve user from auth token")?;
    Ok(response.user.username)
}
