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

    pub fn git_sha_from_object_key(repo_name: &RepoName, object_key: &str) -> Option<String> {
        let prefix = BuildCacheKey::new(repo_name.clone(), "").repo_prefix();
        if !object_key.starts_with(&prefix) {
            return None;
        }
        let rest = &object_key[prefix.len()..];
        let suffix = format!("/{}", Self::CACHE_ARCHIVE_NAME);
        if !rest.ends_with(&suffix) {
            return None;
        }
        let git_sha = &rest[..rest.len().saturating_sub(suffix.len())];
        if git_sha.is_empty() || git_sha.contains('/') {
            return None;
        }
        Some(git_sha.to_string())
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
    fn git_sha_from_object_key_extracts_sha() {
        let repo = RepoName::new("acme", "anvils").expect("repo");
        let key = BuildCacheKey::new(repo.clone(), "deadbeef");

        let extracted =
            BuildCacheKey::git_sha_from_object_key(&repo, &key.object_key()).expect("sha");
        assert_eq!(extracted, "deadbeef");
    }

    #[test]
    fn git_sha_from_object_key_rejects_mismatched_prefix() {
        let repo = RepoName::new("acme", "anvils").expect("repo");
        let other_repo = RepoName::new("acme", "balloons").expect("repo");
        let key = BuildCacheKey::new(other_repo, "deadbeef");

        assert!(BuildCacheKey::git_sha_from_object_key(&repo, &key.object_key()).is_none());
    }
}
