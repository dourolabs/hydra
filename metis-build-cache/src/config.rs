use crate::error::BuildCacheError;
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildCacheConfig {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

impl Default for BuildCacheConfig {
    fn default() -> Self {
        Self {
            include: vec![
                "target/".to_string(),
                "dist/".to_string(),
                "build/".to_string(),
                ".cargo/".to_string(),
                "node_modules/".to_string(),
            ],
            exclude: vec!["*.log".to_string(), "tmp/".to_string(), ".git/".to_string()],
        }
    }
}

impl BuildCacheConfig {
    pub fn matcher(&self) -> Result<BuildCacheMatcher, BuildCacheError> {
        let include = build_glob_set(&self.include)?;
        let exclude = build_glob_set(&self.exclude)?;
        Ok(BuildCacheMatcher {
            include,
            exclude,
            include_is_empty: self.include.is_empty(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct BuildCacheMatcher {
    include: GlobSet,
    exclude: GlobSet,
    include_is_empty: bool,
}

impl BuildCacheMatcher {
    pub fn is_included(&self, path: &Path) -> bool {
        let normalized = normalize_path(path);
        let include_match = if self.include_is_empty {
            true
        } else {
            self.include.is_match(&normalized)
        };
        include_match && !self.exclude.is_match(&normalized)
    }
}

fn build_glob_set(patterns: &[String]) -> Result<GlobSet, BuildCacheError> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let normalized = normalize_glob_pattern(pattern);
        let glob = Glob::new(&normalized).map_err(|err| BuildCacheError::glob(pattern, err))?;
        builder.add(glob);
    }
    builder
        .build()
        .map_err(|err| BuildCacheError::glob("<set>", err))
}

fn normalize_glob_pattern(pattern: &str) -> String {
    let trimmed = pattern
        .trim()
        .trim_start_matches("./")
        .trim_start_matches('/');
    if trimmed.is_empty() {
        return "**/*".to_string();
    }

    let mut normalized = if trimmed.starts_with("**/") {
        trimmed.to_string()
    } else {
        format!("**/{trimmed}")
    };

    if normalized.ends_with('/') {
        normalized.push_str("**");
    }

    normalized
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_includes_expected_patterns() {
        let config = BuildCacheConfig::default();
        assert_eq!(
            config.include,
            vec![
                "target/".to_string(),
                "dist/".to_string(),
                "build/".to_string(),
                ".cargo/".to_string(),
                "node_modules/".to_string(),
            ]
        );
        assert_eq!(
            config.exclude,
            vec!["*.log".to_string(), "tmp/".to_string(), ".git/".to_string()]
        );
    }

    #[test]
    fn matcher_includes_paths_at_any_depth() {
        let config = BuildCacheConfig::default();
        let matcher = config.matcher().expect("matcher");

        assert!(matcher.is_included(Path::new("target/output.o")));
        assert!(matcher.is_included(Path::new("nested/target/output.o")));
        assert!(matcher.is_included(Path::new("nested/.cargo/registry/index")));
        assert!(!matcher.is_included(Path::new("nested/tmp/output.o")));
        assert!(!matcher.is_included(Path::new("nested/.git/config")));
        assert!(!matcher.is_included(Path::new("nested/target/build.log")));
    }

    #[test]
    fn matcher_excludes_take_precedence() {
        let config = BuildCacheConfig {
            include: vec!["target/".to_string()],
            exclude: vec!["tmp/".to_string()],
        };
        let matcher = config.matcher().expect("matcher");

        assert!(matcher.is_included(Path::new("target/output.o")));
        assert!(!matcher.is_included(Path::new("target/tmp/output.o")));
    }
}
