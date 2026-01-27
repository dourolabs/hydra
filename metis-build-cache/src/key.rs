use metis_common::RepoName;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BuildCacheKey {
    pub repo_name: RepoName,
    pub git_sha: String,
}

impl BuildCacheKey {
    const CACHE_ARCHIVE_NAME: &'static str = "cache.tar.zst";
    const REPO_PREFIX_ROOT: &'static str = "repo";

    pub fn new(repo_name: RepoName, git_sha: impl Into<String>) -> Self {
        Self {
            repo_name,
            git_sha: git_sha.into(),
        }
    }

    pub fn object_key(&self) -> String {
        let mut segments = self.prefix_segments();
        segments.push(self.git_sha.clone());
        segments.push(Self::CACHE_ARCHIVE_NAME.to_string());
        segments.join("/")
    }

    pub fn repo_prefix(&self) -> String {
        let mut prefix = self.prefix_segments().join("/");
        prefix.push('/');
        prefix
    }

    fn prefix_segments(&self) -> Vec<String> {
        vec![Self::REPO_PREFIX_ROOT.to_string(), self.repo_name.as_str()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_key_formats_stable_path() {
        let repo = RepoName::new("acme", "anvils").expect("repo");
        let key = BuildCacheKey::new(repo, "deadbeef");

        assert_eq!(key.object_key(), "repo/acme/anvils/deadbeef/cache.tar.zst");
    }

    #[test]
    fn repo_prefix_targets_repo_listing() {
        let repo = RepoName::new("acme", "anvils").expect("repo");
        let key = BuildCacheKey::new(repo, "deadbeef");

        assert_eq!(key.repo_prefix(), "repo/acme/anvils/");
    }
}
