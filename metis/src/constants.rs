//! Constants for file paths and directory names used throughout the metis CLI.
//!
//! These constants centralize path-related strings to ensure consistency
//! and make it easier to maintain and update paths across the codebase.

/// Base directory for CLI assets (`~/.local/share/metis`).
pub const METIS_DIR: &str = "~/.local/share/metis";

/// The default server URL used when no config is provided.
pub const DEFAULT_SERVER_URL: &str = "http://metis-staging.monster-vibes.ts.net";

/// The output text file name (`output.txt`) used when capturing codex output for a job.
pub const OUTPUT_TXT_FILE: &str = "output.txt";

/// The default configuration file path (`~/.local/share/metis/config.toml`) used when no
/// config file is explicitly specified via the `--config` flag.
pub const DEFAULT_CONFIG_FILE: &str = "~/.local/share/metis/config.toml";

/// The default auth token file path (`~/.local/share/metis/auth-token`).
pub const DEFAULT_AUTH_TOKEN_PATH: &str = "~/.local/share/metis/auth-token";
