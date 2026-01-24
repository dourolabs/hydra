use crate::constants::DEFAULT_SERVER_URL;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Deserialize, Serialize)]
pub struct AppConfig {
    pub server: ServerSection,
}

impl AppConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let resolved_path = expand_path(path);
        let contents = fs::read_to_string(&resolved_path).with_context(|| {
            format!(
                "Unable to read configuration file '{}'",
                resolved_path.display()
            )
        })?;
        toml::from_str(&contents)
            .with_context(|| format!("Invalid configuration in '{}'", resolved_path.display()))
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ServerSection {
    pub url: String,
}

/// Expand a leading tilde to the user's home directory.
pub fn expand_path<P: AsRef<Path>>(path: P) -> PathBuf {
    let path = path.as_ref();
    match path.to_str() {
        Some(raw) if raw.starts_with('~') => PathBuf::from(shellexpand::tilde(raw).into_owned()),
        _ => path.to_path_buf(),
    }
}

pub fn default_app_config() -> AppConfig {
    AppConfig {
        server: ServerSection {
            url: DEFAULT_SERVER_URL.to_string(),
        },
    }
}

pub fn create_default_config(resolved_path: &Path) -> Result<()> {
    if let Some(dir) = resolved_path.parent() {
        fs::create_dir_all(dir).with_context(|| {
            format!(
                "failed to create configuration directory '{}'",
                dir.display()
            )
        })?;
    }

    let default_contents = toml::to_string_pretty(&default_app_config())
        .context("failed to serialize default configuration")?;
    fs::write(resolved_path, default_contents).with_context(|| {
        format!(
            "failed to write default configuration to '{}'",
            resolved_path.display()
        )
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::AppConfig;

    #[test]
    fn config_requires_server_url() {
        let err = toml::from_str::<AppConfig>("[server]\n").unwrap_err();
        assert!(err.to_string().contains("missing field `url`"));
    }

    #[test]
    fn config_requires_server_section() {
        let err = toml::from_str::<AppConfig>("").unwrap_err();
        assert!(err.to_string().contains("missing field `server`"));
    }
}
