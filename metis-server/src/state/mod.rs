use std::{collections::HashMap, path::PathBuf};

/// Connection details for a git repository remote.
#[derive(Debug, Clone)]
pub struct GitRepository {
    pub name: String,
    pub remote_url: String,
    pub default_branch: Option<String>,
    pub ssh_key_path: Option<PathBuf>,
    pub username: Option<String>,
    pub password: Option<String>,
}

/// Aggregated state for repositories the service can interact with.
#[derive(Debug, Default)]
pub struct ServiceState {
    pub repositories: HashMap<String, GitRepository>,
}
