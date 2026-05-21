/// Environment variable names used across the hydra server and CLI.
pub const ENV_OPENAI_API_KEY: &str = "OPENAI_API_KEY";
pub const ENV_ANTHROPIC_API_KEY: &str = "ANTHROPIC_API_KEY";
pub const ENV_CLAUDE_CODE_OAUTH_TOKEN: &str = "CLAUDE_CODE_OAUTH_TOKEN";
pub const ENV_HYDRA_CONFIG: &str = "HYDRA_CONFIG";
pub const ENV_HYDRA_DATABASE_URL: &str = "HYDRA_DATABASE_URL";
pub const ENV_DATABASE_URL: &str = "DATABASE_URL";
pub const ENV_HYDRA_SERVER_URL: &str = "HYDRA_SERVER_URL";
pub const ENV_HYDRA_API_ORIGIN: &str = "HYDRA_API_ORIGIN";
pub const ENV_HYDRA_ID: &str = "HYDRA_ID";
pub const ENV_HYDRA_ISSUE_ID: &str = "HYDRA_ISSUE_ID";
pub const ENV_HYDRA_CONVERSATION_ID: &str = "HYDRA_CONVERSATION_ID";
pub const ENV_HYDRA_TOKEN: &str = "HYDRA_TOKEN";
pub const ENV_HYDRA_DOCUMENTS_DIR: &str = "HYDRA_DOCUMENTS_DIR";
pub const ENV_BROWSER: &str = "BROWSER";

/// Overall request timeout for the HydraClient HTTP client, in seconds.
pub const ENV_HYDRA_HTTP_TIMEOUT_SECS: &str = "HYDRA_HTTP_TIMEOUT_SECS";
/// Connect timeout for the HydraClient HTTP client, in seconds.
pub const ENV_HYDRA_HTTP_CONNECT_TIMEOUT_SECS: &str = "HYDRA_HTTP_CONNECT_TIMEOUT_SECS";
/// Idle connection pool timeout for the HydraClient HTTP client, in seconds.
pub const ENV_HYDRA_HTTP_POOL_IDLE_TIMEOUT_SECS: &str = "HYDRA_HTTP_POOL_IDLE_TIMEOUT_SECS";
