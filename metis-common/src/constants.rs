/// Environment variable names used across the metis server and CLI.
pub const ENV_OPENAI_API_KEY: &str = "OPENAI_API_KEY";
pub const ENV_ANTHROPIC_API_KEY: &str = "ANTHROPIC_API_KEY";
pub const ENV_CLAUDE_CODE_OAUTH_TOKEN: &str = "CLAUDE_CODE_OAUTH_TOKEN";
pub const ENV_METIS_CONFIG: &str = "METIS_CONFIG";
pub const ENV_METIS_DATABASE_URL: &str = "METIS_DATABASE_URL";
pub const ENV_DATABASE_URL: &str = "DATABASE_URL";
pub const ENV_METIS_SERVER_URL: &str = "METIS_SERVER_URL";
pub const ENV_METIS_API_ORIGIN: &str = "METIS_API_ORIGIN";
pub const ENV_METIS_ID: &str = "METIS_ID";
pub const ENV_METIS_ISSUE_ID: &str = "METIS_ISSUE_ID";
pub const ENV_METIS_TOKEN: &str = "METIS_TOKEN";
pub const ENV_METIS_DOCUMENTS_DIR: &str = "METIS_DOCUMENTS_DIR";
pub const ENV_BROWSER: &str = "BROWSER";

/// Default CLI configuration file path, shared between the CLI and the server
/// (the server writes auth tokens here during local-auth setup).
pub const DEFAULT_CLI_CONFIG_PATH: &str = "~/.local/share/metis/config.toml";

/// Default data directory for local Metis data (SQLite DB, etc.).
pub const DEFAULT_DATA_DIR: &str = "~/.local/share/metis";
