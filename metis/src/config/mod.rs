use anyhow::{anyhow, bail, Context, Result};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AppConfig {
    pub servers: Vec<ServerSection>,
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
        let parsed = toml::from_str::<AppConfigFile>(&contents)
            .with_context(|| format!("Invalid configuration in '{}'", resolved_path.display()))?;
        parsed.into_app_config()
    }

    pub fn default_server(&self) -> Result<&ServerSection> {
        let mut defaults = self.servers.iter().filter(|server| server.default);
        if let Some(server) = defaults.next() {
            if defaults.next().is_some() {
                return Err(anyhow!(
                    "configuration defines multiple default servers; mark only one as default"
                ));
            }
            return Ok(server);
        }

        if self.servers.len() == 1 {
            return Ok(&self.servers[0]);
        }

        Err(anyhow!(
            "configuration must define a default server or a single server entry"
        ))
    }

    pub fn server_by_url(&self, url: &str) -> Result<Option<&ServerSection>> {
        let target = parse_server_url(url)?;
        for server in &self.servers {
            let server_url = parse_server_url(&server.url)?;
            if server_url == target {
                return Ok(Some(server));
            }
        }
        Ok(None)
    }

    pub fn auth_token_for_url(&self, url: &str) -> Result<Option<&str>> {
        Ok(self
            .server_by_url(url)?
            .and_then(|server| server.auth_token.as_deref()))
    }

    pub fn set_default_server(&mut self, url: &str) -> Result<()> {
        let target = parse_server_url(url)?;
        let has_match = self
            .servers
            .iter()
            .any(|s| parse_server_url(&s.url).ok() == Some(target.clone()));
        if !has_match {
            bail!("no server found matching URL: {url}");
        }
        for server in &mut self.servers {
            let server_url = parse_server_url(&server.url)?;
            server.default = server_url == target;
        }
        Ok(())
    }

    pub fn upsert_server_token(&mut self, url: &str, token: String) -> Result<()> {
        let target = parse_server_url(url)?;
        for server in &mut self.servers {
            let server_url = parse_server_url(&server.url)?;
            if server_url == target {
                server.auth_token = Some(token);
                return Ok(());
            }
        }

        self.servers.push(ServerSection {
            url: url.trim().to_string(),
            auth_token: Some(token),
            default: self.servers.is_empty(),
        });

        Ok(())
    }

    /// Create a config with a single server entry marked as default.
    pub fn single_server(url: &str, auth_token: &str) -> Self {
        AppConfig {
            servers: vec![ServerSection {
                url: url.to_string(),
                auth_token: Some(auth_token.to_string()),
                default: true,
            }],
        }
    }

    pub fn write_to(&self, path: &Path) -> Result<()> {
        let contents = toml::to_string_pretty(self).context("failed to serialize configuration")?;
        fs::write(path, contents)
            .with_context(|| format!("failed to write configuration to '{}'", path.display()))?;
        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ServerSection {
    pub url: String,
    #[serde(default)]
    pub auth_token: Option<String>,
    #[serde(default)]
    pub default: bool,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AppConfigFile {
    Legacy { server: ServerSection },
    Multi { servers: Vec<ServerSection> },
}

impl AppConfigFile {
    fn into_app_config(self) -> Result<AppConfig> {
        match self {
            AppConfigFile::Legacy { mut server } => {
                server.default = true;
                Ok(AppConfig {
                    servers: vec![server],
                })
            }
            AppConfigFile::Multi { servers } => {
                if servers.is_empty() {
                    Err(anyhow!(
                        "configuration must define at least one server entry"
                    ))
                } else {
                    Ok(AppConfig { servers })
                }
            }
        }
    }
}

/// Expand a leading tilde to the user's home directory.
pub fn expand_path<P: AsRef<Path>>(path: P) -> PathBuf {
    let path = path.as_ref();
    match path.to_str() {
        Some(raw) if raw.starts_with('~') => PathBuf::from(shellexpand::tilde(raw).into_owned()),
        _ => path.to_path_buf(),
    }
}

pub fn empty_app_config() -> AppConfig {
    AppConfig { servers: vec![] }
}

pub fn store_auth_token(config_path: &Path, server_url: &str, auth_token: &str) -> Result<()> {
    let resolved_path = expand_path(config_path);
    let mut config = if resolved_path.exists() {
        AppConfig::load(&resolved_path)?
    } else {
        if let Some(dir) = resolved_path.parent() {
            fs::create_dir_all(dir).with_context(|| {
                format!(
                    "failed to create configuration directory '{}'",
                    dir.display()
                )
            })?;
        }
        empty_app_config()
    };
    config.upsert_server_token(server_url, auth_token.to_string())?;
    config.write_to(&resolved_path)?;
    Ok(())
}

fn parse_server_url(url: &str) -> Result<Url> {
    Url::parse(url.trim()).with_context(|| format!("invalid Metis server URL '{url}'"))
}

#[cfg(test)]
mod tests {
    use super::{AppConfig, AppConfigFile, ServerSection};

    #[test]
    fn config_requires_server_url() {
        let err = toml::from_str::<AppConfig>("[[servers]]\n").unwrap_err();
        assert!(err.to_string().contains("missing field `url`"));
    }

    #[test]
    fn config_requires_server_section() {
        let err = toml::from_str::<AppConfig>("").unwrap_err();
        assert!(err.to_string().contains("missing field `servers`"));
    }

    #[test]
    fn config_legacy_sections_become_default() {
        let config = toml::from_str::<AppConfigFile>("[server]\nurl = \"http://localhost:8080\"\n")
            .expect("parse legacy config")
            .into_app_config()
            .expect("convert legacy config");
        let server = config.default_server().expect("default server");
        assert_eq!(server.url, "http://localhost:8080");
        assert!(server.default);
    }

    #[test]
    fn config_default_server_falls_back_to_single_entry() {
        let config = AppConfig {
            servers: vec![ServerSection {
                url: "http://localhost:8080".to_string(),
                auth_token: None,
                default: false,
            }],
        };
        let server = config.default_server().expect("default server");
        assert_eq!(server.url, "http://localhost:8080");
    }

    #[test]
    fn set_default_server_marks_correct_server() {
        let mut config = AppConfig {
            servers: vec![
                ServerSection {
                    url: "http://staging.example.com".to_string(),
                    auth_token: None,
                    default: true,
                },
                ServerSection {
                    url: "http://127.0.0.1:8080".to_string(),
                    auth_token: Some("tok".to_string()),
                    default: false,
                },
            ],
        };

        config
            .set_default_server("http://127.0.0.1:8080")
            .expect("set default");

        assert!(!config.servers[0].default);
        assert!(config.servers[1].default);

        let default = config.default_server().expect("default server");
        assert_eq!(default.url, "http://127.0.0.1:8080");
    }

    #[test]
    fn set_default_server_errors_on_unknown_url() {
        let mut config = AppConfig {
            servers: vec![ServerSection {
                url: "http://staging.example.com".to_string(),
                auth_token: None,
                default: true,
            }],
        };

        let err = config
            .set_default_server("http://nonexistent.example.com")
            .unwrap_err();
        assert!(err
            .to_string()
            .contains("no server found matching URL: http://nonexistent.example.com"),);
        // Original server should still be default (unchanged)
        assert!(config.servers[0].default);
    }

    #[test]
    fn config_auth_token_matches_normalized_url() {
        let config = AppConfig {
            servers: vec![ServerSection {
                url: "http://localhost:8080".to_string(),
                auth_token: Some("token-123".to_string()),
                default: true,
            }],
        };

        let token = config
            .auth_token_for_url("http://localhost:8080/")
            .expect("lookup token");
        assert_eq!(token, Some("token-123"));
    }
}
