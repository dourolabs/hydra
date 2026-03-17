use std::io;

#[derive(Debug, thiserror::Error)]
pub enum BuildCacheError {
    #[error("I/O error while {context}: {source}")]
    Io {
        context: &'static str,
        source: io::Error,
    },
    #[error("invalid glob pattern '{pattern}': {source}")]
    Glob {
        pattern: String,
        source: globset::Error,
    },
    #[error("invalid config for {field}: {message}")]
    Config {
        field: &'static str,
        message: String,
    },
    #[error("storage error while {context}: {message}")]
    Storage {
        context: &'static str,
        message: String,
    },
    #[error("git error while {context}: {source}")]
    Git {
        context: &'static str,
        source: git2::Error,
    },
    #[error("cache apply would overwrite {count} tracked file(s): {sample}")]
    TrackedFiles { count: usize, sample: String },
}

impl BuildCacheError {
    pub fn io(context: &'static str, source: io::Error) -> Self {
        Self::Io { context, source }
    }

    pub fn glob(pattern: impl Into<String>, source: globset::Error) -> Self {
        Self::Glob {
            pattern: pattern.into(),
            source,
        }
    }

    pub fn config(field: &'static str, message: impl Into<String>) -> Self {
        Self::Config {
            field,
            message: message.into(),
        }
    }

    pub fn storage(context: &'static str, message: impl Into<String>) -> Self {
        Self::Storage {
            context,
            message: message.into(),
        }
    }

    pub fn git(context: &'static str, source: git2::Error) -> Self {
        Self::Git { context, source }
    }

    pub fn tracked_files(conflicts: &[std::path::PathBuf]) -> Self {
        let count = conflicts.len();
        let mut sample = conflicts
            .iter()
            .take(5)
            .map(|path| path.to_string_lossy())
            .collect::<Vec<_>>()
            .join(", ");
        if count > 5 {
            sample.push_str(&format!(" ({} more)", count - 5));
        }
        Self::TrackedFiles { count, sample }
    }
}
