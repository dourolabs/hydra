#![allow(dead_code)]

use git2::{Commit, MergeOptions, Oid, Repository, Signature};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchEntry {
    pub commit: Oid,
    pub summary: Option<String>,
}

pub struct MergeQueueImpl {
    repo: Repository,
    signature: Signature<'static>,
    base_ref: String,
    base: Oid,
    tip: Oid,
    patches: Vec<PatchEntry>,
}

#[derive(Debug, Error)]
pub enum MergeQueueError {
    #[error("patch {0} could not be merged")]
    Unmergeable(Oid),
    #[error(transparent)]
    Git(#[from] git2::Error),
}

impl MergeQueueImpl {
    pub fn new(repo: Repository, base_ref: impl Into<String>) -> Result<Self, git2::Error> {
        let base_ref = base_ref.into();
        let base = {
            let base_commit = repo.revparse_single(&base_ref)?.peel_to_commit()?;
            base_commit.id()
        };
        let signature = Signature::now("metis-merge-queue", "merge-queue@metis.local")?;

        Ok(Self {
            repo,
            signature,
            base_ref,
            base,
            tip: base,
            patches: Vec::new(),
        })
    }

    pub fn base_ref(&self) -> &str {
        &self.base_ref
    }

    pub fn base(&self) -> Oid {
        self.base
    }

    pub fn tip(&self) -> Oid {
        self.tip
    }

    pub fn patches(&self) -> &[PatchEntry] {
        &self.patches
    }

    pub fn append(&mut self, patch: PatchEntry) -> Result<(), MergeQueueError> {
        let patch_commit = self.repo.find_commit(patch.commit)?;
        let new_tip = self.merge_patch(self.tip, &patch_commit)?;

        self.tip = new_tip;
        self.patches.push(patch);

        Ok(())
    }

    pub fn evict(&mut self, commit_id: Oid) -> Result<Vec<PatchEntry>, MergeQueueError> {
        if !self.patches.iter().any(|entry| entry.commit == commit_id) {
            return Ok(Vec::new());
        }

        let existing = std::mem::take(&mut self.patches);
        let mut current_tip = self.base;
        let mut kept = Vec::new();
        let mut evicted = Vec::new();

        for patch in existing {
            if patch.commit == commit_id {
                evicted.push(patch);
                continue;
            }

            let patch_commit = self.repo.find_commit(patch.commit)?;
            match self.merge_patch(current_tip, &patch_commit) {
                Ok(new_tip) => {
                    current_tip = new_tip;
                    kept.push(patch);
                }
                Err(MergeQueueError::Unmergeable(_)) => {
                    evicted.push(patch);
                }
                Err(err) => {
                    self.patches = kept;
                    self.tip = current_tip;
                    return Err(err);
                }
            }
        }

        self.tip = current_tip;
        self.patches = kept;

        Ok(evicted)
    }

    fn merge_patch(
        &self,
        current_tip: Oid,
        patch_commit: &Commit<'_>,
    ) -> Result<Oid, MergeQueueError> {
        let tip_commit = self.repo.find_commit(current_tip)?;
        let merge_options = MergeOptions::new();
        let mut index = self
            .repo
            .merge_commits(&tip_commit, patch_commit, Some(&merge_options))?;

        if index.has_conflicts() {
            return Err(MergeQueueError::Unmergeable(patch_commit.id()));
        }

        let tree_oid = index.write_tree_to(&self.repo)?;
        let tree = self.repo.find_tree(tree_oid)?;
        let message = format!("Merge {} into {}", patch_commit.id(), current_tip);

        let new_tip = self.repo.commit(
            None,
            &self.signature,
            &self.signature,
            &message,
            &tree,
            &[&tip_commit, patch_commit],
        )?;

        Ok(new_tip)
    }
}

#[cfg(test)]
mod tests {
    use super::{MergeQueueError, MergeQueueImpl, PatchEntry};
    use anyhow::Result;
    use git2::{Oid, Repository, ResetType, Signature};
    use std::{fs, path::Path};
    use tempfile::TempDir;

    const FILE_PATH: &str = "file.txt";
    const BASE_REF: &str = "refs/heads/base";

    #[test]
    fn append_advances_tip_and_records_patches() -> Result<()> {
        let (tempdir, repo) = repo_with_base("base\n", "base updated\n")?;
        let base_commit = resolve_oid(&repo, BASE_REF)?;

        let patch1 = commit_with_parent(&repo, base_commit, "from patch1\n", "patch1")?;
        let patch2 = commit_with_parent(&repo, patch1, "from patch2\n", "patch2")?;

        let queue_repo = Repository::open(tempdir.path())?;
        let mut queue = MergeQueueImpl::new(queue_repo, BASE_REF)?;

        queue.append(PatchEntry {
            commit: patch1,
            summary: Some("first patch".to_string()),
        })?;
        queue.append(PatchEntry {
            commit: patch2,
            summary: None,
        })?;

        assert_eq!(queue.patches().len(), 2);
        assert_eq!(queue.base(), base_commit);
        assert_ne!(queue.tip(), queue.base());
        assert_eq!(file_at_commit(&repo, queue.tip())?, "from patch2\n");

        Ok(())
    }

    #[test]
    fn append_rejects_unmergeable_patch() -> Result<()> {
        let (tempdir, repo) = repo_with_base("base\n", "base updated\n")?;
        let base_commit = resolve_oid(&repo, BASE_REF)?;

        let patch1 = commit_with_parent(&repo, base_commit, "left change\n", "patch1")?;
        let patch2 = commit_with_parent(&repo, base_commit, "right change\n", "patch2")?;

        let queue_repo = Repository::open(tempdir.path())?;
        let mut queue = MergeQueueImpl::new(queue_repo, BASE_REF)?;

        queue.append(PatchEntry {
            commit: patch1,
            summary: None,
        })?;
        let tip_after_first = queue.tip();

        let result = queue.append(PatchEntry {
            commit: patch2,
            summary: Some("conflicting patch".to_string()),
        });

        assert!(matches!(
            result,
            Err(MergeQueueError::Unmergeable(id)) if id == patch2
        ));
        assert_eq!(queue.tip(), tip_after_first);
        assert_eq!(queue.patches().len(), 1);

        Ok(())
    }

    #[test]
    fn evict_removes_patch_and_reapplies_queue() -> Result<()> {
        let tempdir = TempDir::new()?;
        let repo = Repository::init(tempdir.path())?;

        let root = initial_commit(&repo, "initial\n")?;
        let base = commit_with_parent(&repo, root, "base branch\n", "base")?;
        repo.reference(BASE_REF, base, true, "set base ref")?;

        let patch1 = commit_with_parent(&repo, base, "aligned value\n", "patch1")?;

        repo.reset(&repo.find_object(root, None)?, ResetType::Hard, None)?;
        let patch2 = commit_with_parent(&repo, root, "aligned value\n", "patch2")?;

        let queue_repo = Repository::open(tempdir.path())?;
        let mut queue = MergeQueueImpl::new(queue_repo, BASE_REF)?;

        queue.append(PatchEntry {
            commit: patch1,
            summary: Some("kept patch".to_string()),
        })?;
        queue.append(PatchEntry {
            commit: patch2,
            summary: Some("dependent patch".to_string()),
        })?;

        let evicted = queue.evict(patch1)?;

        assert_eq!(evicted.len(), 2);
        assert!(evicted.iter().any(|entry| entry.commit == patch1));
        assert!(evicted.iter().any(|entry| entry.commit == patch2));
        assert_eq!(queue.patches().len(), 0);
        assert_eq!(queue.tip(), base);

        Ok(())
    }

    fn repo_with_base(initial: &str, base_contents: &str) -> Result<(TempDir, Repository)> {
        let tempdir = TempDir::new()?;
        let repo = Repository::init(tempdir.path())?;

        let root = initial_commit(&repo, initial)?;
        let base = commit_with_parent(&repo, root, base_contents, "base")?;
        repo.reference(BASE_REF, base, true, "set base ref")?;

        Ok((tempdir, repo))
    }

    fn initial_commit(repo: &Repository, contents: &str) -> Result<Oid> {
        let mut index = repo.index()?;
        write_file(repo, contents)?;
        index.add_path(Path::new(FILE_PATH))?;
        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        let signature = test_signature()?;

        repo.commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[])
            .map_err(Into::into)
    }

    fn commit_with_parent(
        repo: &Repository,
        parent: Oid,
        contents: &str,
        message: &str,
    ) -> Result<Oid> {
        repo.reset(&repo.find_object(parent, None)?, ResetType::Hard, None)?;
        write_file(repo, contents)?;
        let mut index = repo.index()?;
        index.add_path(Path::new(FILE_PATH))?;
        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        let signature = test_signature()?;
        let parent_commit = repo.find_commit(parent)?;

        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &[&parent_commit],
        )
        .map_err(Into::into)
    }

    fn write_file(repo: &Repository, contents: &str) -> Result<()> {
        let path = repo
            .workdir()
            .map(|dir| dir.join(FILE_PATH))
            .ok_or_else(|| anyhow::anyhow!("repository is bare"))?;
        fs::write(path, contents)?;
        Ok(())
    }

    fn resolve_oid(repo: &Repository, reference: &str) -> Result<Oid> {
        Ok(repo.revparse_single(reference)?.peel_to_commit()?.id())
    }

    fn file_at_commit(repo: &Repository, commit: Oid) -> Result<String> {
        let commit = repo.find_commit(commit)?;
        let tree = commit.tree()?;
        let entry = tree.get_path(Path::new(FILE_PATH))?;
        let blob = repo.find_blob(entry.id())?;

        Ok(String::from_utf8_lossy(blob.content()).into_owned())
    }

    fn test_signature() -> Result<Signature<'static>> {
        Signature::now("metis", "metis@example.com").map_err(Into::into)
    }
}
