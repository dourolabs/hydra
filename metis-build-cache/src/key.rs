use metis_common::RepoName;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BuildCacheKey {
    pub repo_name: RepoName,
    pub git_sha: String,
}

impl BuildCacheKey {
    pub fn new(repo_name: RepoName, git_sha: impl Into<String>) -> Self {
        Self {
            repo_name,
            git_sha: git_sha.into(),
        }
    }
}
