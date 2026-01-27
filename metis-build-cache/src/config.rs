use crate::error::BuildCacheError;
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildCacheConfig {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3StorageConfig {
    pub endpoint_url: String,
    pub bucket: String,
    pub region: String,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub session_token: Option<String>,
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

impl S3StorageConfig {
    pub fn validate(&self) -> Result<(), BuildCacheError> {
        validate_required("endpoint_url", &self.endpoint_url)?;
        validate_required("bucket", &self.bucket)?;
        validate_required("region", &self.region)?;
        validate_optional_non_empty("access_key_id", &self.access_key_id)?;
        validate_optional_non_empty("secret_access_key", &self.secret_access_key)?;
        validate_optional_non_empty("session_token", &self.session_token)?;

        let has_access = self.access_key_id.as_ref().is_some();
        let has_secret = self.secret_access_key.as_ref().is_some();

        if has_access ^ has_secret {
            return Err(BuildCacheError::config(
                "credentials",
                "access_key_id and secret_access_key must be provided together",
            ));
        }

        if self.session_token.is_some() && !(has_access && has_secret) {
            return Err(BuildCacheError::config(
                "session_token",
                "session_token requires access_key_id and secret_access_key",
            ));
        }

        Ok(())
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

fn validate_required(field: &'static str, value: &str) -> Result<(), BuildCacheError> {
    if value.trim().is_empty() {
        return Err(BuildCacheError::config(field, "must not be empty"));
    }
    Ok(())
}

fn validate_optional_non_empty(
    field: &'static str,
    value: &Option<String>,
) -> Result<(), BuildCacheError> {
    if let Some(value) = value {
        if value.trim().is_empty() {
            return Err(BuildCacheError::config(field, "must not be empty"));
        }
    }
    Ok(())
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

    #[test]
    fn s3_config_accepts_empty_credentials() {
        let config = S3StorageConfig {
            endpoint_url: "https://s3.example.com".to_string(),
            bucket: "metis-cache".to_string(),
            region: "us-east-1".to_string(),
            access_key_id: None,
            secret_access_key: None,
            session_token: None,
        };

        assert!(config.validate().is_ok());
    }

    #[test]
    fn s3_config_rejects_missing_required_fields() {
        let config = S3StorageConfig {
            endpoint_url: "".to_string(),
            bucket: "metis-cache".to_string(),
            region: "us-east-1".to_string(),
            access_key_id: None,
            secret_access_key: None,
            session_token: None,
        };

        assert!(matches!(
            config.validate(),
            Err(BuildCacheError::Config {
                field: "endpoint_url",
                ..
            })
        ));
    }

    #[test]
    fn s3_config_requires_full_credentials() {
        let config = S3StorageConfig {
            endpoint_url: "https://s3.example.com".to_string(),
            bucket: "metis-cache".to_string(),
            region: "us-east-1".to_string(),
            access_key_id: Some("access".to_string()),
            secret_access_key: None,
            session_token: None,
        };

        assert!(matches!(
            config.validate(),
            Err(BuildCacheError::Config {
                field: "credentials",
                ..
            })
        ));
    }

    #[test]
    fn s3_config_rejects_empty_optional_values() {
        let config = S3StorageConfig {
            endpoint_url: "https://s3.example.com".to_string(),
            bucket: "metis-cache".to_string(),
            region: "us-east-1".to_string(),
            access_key_id: Some("".to_string()),
            secret_access_key: Some("secret".to_string()),
            session_token: None,
        };

        assert!(matches!(
            config.validate(),
            Err(BuildCacheError::Config {
                field: "access_key_id",
                ..
            })
        ));
    }

    #[test]
    fn s3_config_rejects_session_token_without_credentials() {
        let config = S3StorageConfig {
            endpoint_url: "https://s3.example.com".to_string(),
            bucket: "metis-cache".to_string(),
            region: "us-east-1".to_string(),
            access_key_id: None,
            secret_access_key: None,
            session_token: Some("token".to_string()),
        };

        assert!(matches!(
            config.validate(),
            Err(BuildCacheError::Config {
                field: "session_token",
                ..
            })
        ));
    }
}
