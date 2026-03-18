/// Environment variable names used across the hydra server and CLI.
/// Note: The runtime env var names still use METIS_* for backwards compatibility
/// with scripts, deployments, and frontend code. Only the Rust const names
/// have been renamed to HYDRA_*.
pub const ENV_OPENAI_API_KEY: &str = "OPENAI_API_KEY";
pub const ENV_ANTHROPIC_API_KEY: &str = "ANTHROPIC_API_KEY";
pub const ENV_CLAUDE_CODE_OAUTH_TOKEN: &str = "CLAUDE_CODE_OAUTH_TOKEN";
pub const ENV_HYDRA_CONFIG: &str = "METIS_CONFIG";
pub const ENV_HYDRA_DATABASE_URL: &str = "METIS_DATABASE_URL";
pub const ENV_DATABASE_URL: &str = "DATABASE_URL";
pub const ENV_HYDRA_SERVER_URL: &str = "METIS_SERVER_URL";
pub const ENV_HYDRA_API_ORIGIN: &str = "METIS_API_ORIGIN";
pub const ENV_HYDRA_ID: &str = "METIS_ID";
pub const ENV_HYDRA_ISSUE_ID: &str = "METIS_ISSUE_ID";
pub const ENV_HYDRA_TOKEN: &str = "METIS_TOKEN";
pub const ENV_HYDRA_DOCUMENTS_DIR: &str = "METIS_DOCUMENTS_DIR";
pub const ENV_BROWSER: &str = "BROWSER";
