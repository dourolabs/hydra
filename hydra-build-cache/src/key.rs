use hydra_common::RepoName;

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

    pub fn from_object_key(object_key: &str) -> Option<Self> {
        let mut segments = object_key.split('/');
        let root = segments.next()?;
        if root != Self::REPO_PREFIX_ROOT {
            return None;
        }
        let organization = segments.next()?;
        let repo = segments.next()?;
        let git_sha = segments.next()?;
        let archive = segments.next()?;
        if segments.next().is_some() {
            return None;
        }
        if archive != Self::CACHE_ARCHIVE_NAME {
            return None;
        }
        let repo_name = RepoName::new(organization, repo).ok()?;
        Some(Self::new(repo_name, git_sha))
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

    #[test]
    fn from_object_key_extracts_repo_and_sha() {
        let repo = RepoName::new("acme", "anvils").expect("repo");
        let key = BuildCacheKey::new(repo.clone(), "deadbeef");

        let extracted = BuildCacheKey::from_object_key(&key.object_key()).expect("key");
        assert_eq!(extracted, key);
    }

    #[test]
    fn from_object_key_rejects_unknown_prefix() {
        let key = "cache/acme/anvils/deadbeef/cache.tar.zst";

        assert!(BuildCacheKey::from_object_key(key).is_none());
    }
}
