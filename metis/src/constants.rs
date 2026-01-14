//! Constants for file paths and directory names used throughout the metis CLI.
//!
//! These constants centralize path-related strings to ensure consistency
//! and make it easier to maintain and update paths across the codebase.

/// The main metis directory name (`.metis`) used to store metis-specific files
/// in the working directory. This directory contains subdirectories like
/// `output`.
pub const METIS_DIR: &str = ".metis";

/// The output subdirectory name (`output`) under `.metis` where job outputs
/// are stored, including files like `output.txt` and `changes.patch`.
pub const OUTPUT_DIR: &str = "output";

/// The output text file name (`output.txt`) stored in the output directory,
/// containing the last message output from a job execution.
pub const OUTPUT_TXT_FILE: &str = "output.txt";

/// The default configuration file name (`config.toml`) used when no config
/// file is explicitly specified via the `--config` flag.
pub const DEFAULT_CONFIG_FILE: &str = "config.toml";
