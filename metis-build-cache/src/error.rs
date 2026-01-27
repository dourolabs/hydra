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
}
