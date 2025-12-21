/// Constants for file paths and directory names used throughout the metis CLI.
///
/// These constants centralize path-related strings to ensure consistency
/// and make it easier to maintain and update paths across the codebase.

/// The main metis directory name (`.metis`) used to store metis-specific files
/// in the working directory. This directory contains subdirectories like
/// `output` and `parents`.
pub const METIS_DIR: &str = ".metis";

/// The output subdirectory name (`output`) under `.metis` where job outputs
/// are stored, including files like `output.txt` and `changes.patch`.
pub const OUTPUT_DIR: &str = "output";

/// The parents subdirectory name (`parents`) under `.metis` where parent
/// job contexts are stored.
pub const PARENTS_DIR: &str = "parents";

/// The output text file name (`output.txt`) stored in the output directory,
/// containing the last message output from a job execution.
pub const OUTPUT_TXT_FILE: &str = "output.txt";

/// The patch file name (`changes.patch`) stored in the output directory,
/// containing the git diff of changes made during job execution.
pub const CHANGES_PATCH_FILE: &str = "changes.patch";

/// The default configuration file name (`config.toml`) used when no config
/// file is explicitly specified via the `--config` flag.
pub const DEFAULT_CONFIG_FILE: &str = "config.toml";

/// Expression depth limits used when configuring the Rhai engine.
pub const RHAI_MAX_EXPR_DEPTHS: (usize, usize) = (256, 256);

/// Maximum call stack depth for the Rhai engine.
pub const RHAI_MAX_CALL_LEVELS: usize = 128;

/// Maximum number of operations a Rhai script may execute.
pub const RHAI_MAX_OPERATIONS: u64 = 50_000;
