use anyhow::{Context, Result, ensure};
use metis_common::constants::ENV_METIS_CONFIG;
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    #[serde(default)]
    pub server: ServerSection,
    pub storage: StorageSection,
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
        let config: Self = toml::from_str(&contents)
            .with_context(|| format!("Invalid configuration in '{}'", resolved_path.display()))?;
        config
            .validate()
            .with_context(|| format!("Invalid configuration in '{}'", resolved_path.display()))?;
        Ok(config)
    }

    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.server.bind_host, self.server.bind_port)
    }

    pub fn storage_root(&self) -> PathBuf {
        expand_path(&self.storage.root_dir)
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            non_empty(&self.storage.root_dir).is_some(),
            "storage.root_dir must be set"
        );
        ensure!(
            non_empty(&self.server.bind_host).is_some(),
            "server.bind_host must be set"
        );
        ensure!(self.server.bind_port > 0, "server.bind_port must be set");
        ensure!(
            self.server.request_body_limit_bytes > 0,
            "server.request_body_limit_bytes must be set"
        );
        Ok(())
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerSection {
    #[serde(default = "default_bind_host")]
    pub bind_host: String,
    #[serde(default = "default_bind_port")]
    pub bind_port: u16,
    #[serde(default = "default_request_body_limit_bytes")]
    pub request_body_limit_bytes: usize,
}

impl Default for ServerSection {
    fn default() -> Self {
        Self {
            bind_host: default_bind_host(),
            bind_port: default_bind_port(),
            request_body_limit_bytes: default_request_body_limit_bytes(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct StorageSection {
    pub root_dir: String,
}

pub fn config_path() -> PathBuf {
    std::env::var(ENV_METIS_CONFIG)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("config.toml"))
}

pub(crate) fn expand_path<P: AsRef<Path>>(path: P) -> PathBuf {
    let path = path.as_ref();
    match path.to_str() {
        Some(raw) if raw.starts_with('~') => PathBuf::from(shellexpand::tilde(raw).into_owned()),
        _ => path.to_path_buf(),
    }
}

pub(crate) fn non_empty(value: &str) -> Option<&str> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value.trim())
    }
}

fn default_bind_host() -> String {
    "0.0.0.0".to_string()
}

const fn default_bind_port() -> u16 {
    9090
}

const fn default_request_body_limit_bytes() -> usize {
    134_217_728
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_config_from_file() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let path = temp_dir.path().join("config.toml");
        fs::write(
            &path,
            r#"
[server]
bind_host = "127.0.0.1"
bind_port = 9091
request_body_limit_bytes = 1024

[storage]
root_dir = "~/metis-s3"
"#,
        )?;
        let config = AppConfig::load(&path)?;
        assert_eq!(config.server.bind_host, "127.0.0.1");
        assert_eq!(config.server.bind_port, 9091);
        assert_eq!(config.server.request_body_limit_bytes, 1024);
        assert!(config.storage_root().to_string_lossy().contains("metis-s3"));
        Ok(())
    }
}
