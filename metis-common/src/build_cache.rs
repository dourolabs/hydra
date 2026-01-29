use serde::{Deserialize, Serialize};

pub const DEFAULT_BUILD_CACHE_INCLUDE: [&str; 5] =
    ["target/", "dist/", "build/", ".cargo/", "node_modules/"];
pub const DEFAULT_BUILD_CACHE_EXCLUDE: [&str; 3] = ["*.log", "tmp/", ".git/"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildCacheSettings {
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_entries_per_repo: Option<usize>,
}

impl Default for BuildCacheSettings {
    fn default() -> Self {
        Self {
            include: default_build_cache_include(),
            exclude: default_build_cache_exclude(),
            max_entries_per_repo: None,
        }
    }
}

pub fn default_build_cache_include() -> Vec<String> {
    DEFAULT_BUILD_CACHE_INCLUDE
        .iter()
        .map(|value| (*value).to_string())
        .collect()
}

pub fn default_build_cache_exclude() -> Vec<String> {
    DEFAULT_BUILD_CACHE_EXCLUDE
        .iter()
        .map(|value| (*value).to_string())
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BuildCacheStorageConfig {
    #[serde(rename = "filesystem")]
    FileSystem { root_dir: String },
    #[serde(rename = "s3")]
    S3 {
        endpoint_url: String,
        bucket: String,
        region: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        access_key_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        secret_access_key: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_token: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildCacheContext {
    pub storage: BuildCacheStorageConfig,
    #[serde(default)]
    pub settings: BuildCacheSettings,
}
