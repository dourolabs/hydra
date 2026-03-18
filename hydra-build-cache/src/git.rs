use crate::BuildCacheEntry;
use crate::error::BuildCacheError;
use crate::key::BuildCacheKey;
use git2::{ErrorCode, Oid, Repository};
use hydra_common::RepoName;
use std::path::Path;
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct NearestCacheEntry {
    pub key: BuildCacheKey,
    pub entry: BuildCacheEntry,
    pub distance: usize,
}

pub fn find_nearest_cache_entry(
    repo_root: impl AsRef<Path>,
    repo_name: RepoName,
    entries: impl IntoIterator<Item = BuildCacheEntry>,
) -> Result<Option<NearestCacheEntry>, BuildCacheError> {
    let repo = Repository::discover(repo_root.as_ref())
        .map_err(|err| BuildCacheError::git("discovering git repository", err))?;
    let head_commit = repo
        .head()
        .map_err(|err| BuildCacheError::git("resolving HEAD", err))?
        .peel_to_commit()
        .map_err(|err| BuildCacheError::git("peeling HEAD to commit", err))?;

    let mut best: Option<NearestCacheEntry> = None;
    for entry in entries {
        let Some(cache_key) = BuildCacheKey::from_object_key(&entry.key) else {
            continue;
        };
        if cache_key.repo_name != repo_name {
            continue;
        }
        let Ok(oid) = Oid::from_str(&cache_key.git_sha) else {
            continue;
        };
        if let Err(err) = repo.find_commit(oid) {
            if err.code() == ErrorCode::NotFound {
                continue;
            }
            return Err(BuildCacheError::git("resolving cache commit", err));
        }

        let distance = match repo.graph_ahead_behind(head_commit.id(), oid) {
            Ok((ahead, behind)) => ahead + behind,
            Err(err) => {
                return Err(BuildCacheError::git("calculating commit distance", err));
            }
        };

        let candidate = NearestCacheEntry {
            key: cache_key,
            entry,
            distance,
        };

        if is_better_candidate(&candidate, best.as_ref()) {
            best = Some(candidate);
        }
    }

    Ok(best)
}

fn is_better_candidate(candidate: &NearestCacheEntry, best: Option<&NearestCacheEntry>) -> bool {
    let Some(best) = best else {
        return true;
    };

    if candidate.distance != best.distance {
        return candidate.distance < best.distance;
    }

    let candidate_time = candidate
        .entry
        .last_modified
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let best_time = best.entry.last_modified.unwrap_or(SystemTime::UNIX_EPOCH);
    if candidate_time != best_time {
        return candidate_time > best_time;
    }

    candidate.entry.key < best.entry.key
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Signature;
    use std::fs;
    use tempfile::tempdir;

    fn init_repo() -> (tempfile::TempDir, Repository, Signature<'static>) {
        let dir = tempdir().expect("tempdir");
        let repo = Repository::init(dir.path()).expect("init repo");
        let signature = Signature::now("hydra", "hydra@example.com").expect("signature");
        (dir, repo, signature)
    }

    fn commit_file(
        repo: &Repository,
        signature: &Signature<'_>,
        path: &Path,
        contents: &str,
        message: &str,
    ) -> Oid {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create dirs");
        }
        fs::write(path, contents).expect("write file");

        let mut index = repo.index().expect("index");
        index
            .add_path(
                path.strip_prefix(repo.path().parent().expect("workdir"))
                    .expect("strip"),
            )
            .expect("add path");
        let tree_id = index.write_tree().expect("write tree");
        let tree = repo.find_tree(tree_id).expect("find tree");
        let parents = repo
            .head()
            .ok()
            .and_then(|head| head.target())
            .and_then(|oid| repo.find_commit(oid).ok());
        let parent_refs = parents.iter().collect::<Vec<_>>();
        repo.commit(
            Some("HEAD"),
            signature,
            signature,
            message,
            &tree,
            &parent_refs,
        )
        .expect("commit")
    }

    #[test]
    fn nearest_cache_prefers_closest_commit() {
        let (dir, repo, signature) = init_repo();
        let root = dir.path();
        let workdir = repo.path().parent().expect("workdir");
        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout.force();

        let base = commit_file(
            &repo,
            &signature,
            &workdir.join("README.md"),
            "base",
            "base",
        );
        let head = commit_file(
            &repo,
            &signature,
            &workdir.join("src/lib.rs"),
            "head",
            "head",
        );

        let branch = repo
            .branch(
                "feature",
                &repo.find_commit(base).expect("base commit"),
                false,
            )
            .expect("branch");
        let feature_commit = {
            repo.set_head(branch.get().name().expect("branch name"))
                .expect("set head");
            repo.checkout_head(Some(&mut checkout)).expect("checkout");
            commit_file(
                &repo,
                &signature,
                &workdir.join("feature.txt"),
                "feature",
                "feature",
            )
        };

        repo.set_head_detached(head).expect("restore head");
        repo.checkout_head(Some(&mut checkout))
            .expect("checkout head");

        let repo_name = RepoName::new("acme", "anvils").expect("repo name");
        let entries = vec![
            BuildCacheEntry {
                key: BuildCacheKey::new(repo_name.clone(), base.to_string()).object_key(),
                last_modified: None,
            },
            BuildCacheEntry {
                key: BuildCacheKey::new(repo_name.clone(), feature_commit.to_string()).object_key(),
                last_modified: None,
            },
            BuildCacheEntry {
                key: BuildCacheKey::new(repo_name.clone(), head.to_string()).object_key(),
                last_modified: None,
            },
        ];

        let nearest = find_nearest_cache_entry(root, repo_name, entries).expect("nearest entry");
        let nearest = nearest.expect("expected nearest");

        assert_eq!(nearest.key.git_sha, head.to_string());
        assert_eq!(nearest.distance, 0);
    }

    #[test]
    fn nearest_cache_skips_unknown_commits() {
        let (dir, repo, signature) = init_repo();
        let root = dir.path();
        let workdir = repo.path().parent().expect("workdir");

        let head = commit_file(
            &repo,
            &signature,
            &workdir.join("README.md"),
            "head",
            "head",
        );

        let repo_name = RepoName::new("acme", "anvils").expect("repo name");
        let entries = vec![
            BuildCacheEntry {
                key: BuildCacheKey::new(repo_name.clone(), "deadbeef").object_key(),
                last_modified: None,
            },
            BuildCacheEntry {
                key: BuildCacheKey::new(repo_name.clone(), head.to_string()).object_key(),
                last_modified: None,
            },
        ];

        let nearest = find_nearest_cache_entry(root, repo_name, entries).expect("nearest entry");
        let nearest = nearest.expect("expected nearest");

        assert_eq!(nearest.key.git_sha, head.to_string());
    }
}
