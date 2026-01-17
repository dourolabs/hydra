//! Constants for file paths and directory names used throughout the metis CLI.
//!
//! These constants centralize path-related strings to ensure consistency
//! and make it easier to maintain and update paths across the codebase.

/// The output text file name (`output.txt`) used when capturing codex output for a job.
pub const OUTPUT_TXT_FILE: &str = "output.txt";

/// The default configuration file name (`config.toml`) used when no config
/// file is explicitly specified via the `--config` flag.
pub const DEFAULT_CONFIG_FILE: &str = "config.toml";
