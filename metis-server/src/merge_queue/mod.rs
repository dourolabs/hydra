#![allow(dead_code)]

use git2::{Commit, Diff, MergeOptions, Oid, Repository, Signature, Time};
use metis_common::PatchId;
use thiserror::Error;

#[derive(Clone, Debug)]
pub struct SignatureInfo {
    pub name: String,
    pub email: String,
    pub time: Time,
}

impl SignatureInfo {
    pub fn from_signature(signature: &Signature<'_>) -> Self {
        Self {
            name: signature.name().unwrap_or_default().to_string(),
            email: signature.email().unwrap_or_default().to_string(),
            time: signature.when(),
        }
    }

    pub fn to_signature(&self) -> Result<Signature<'static>, git2::Error> {
        Signature::new(&self.name, &self.email, &self.time)
    }
}

#[derive(Clone)]
pub struct PatchEntry {
    pub patch_id: PatchId,
    pub commit: Oid,
    pub queued_commit: Oid,
    pub summary: Option<String>,
    pub author: SignatureInfo,
    pub committer: SignatureInfo,
}

impl std::fmt::Debug for PatchEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PatchEntry")
            .field("patch_id", &self.patch_id)
            .field("commit", &self.commit)
            .field("queued_commit", &self.queued_commit)
            .field("summary", &self.summary)
            .finish()
    }
}

#[derive(Debug)]
pub struct MergeQueueImpl {
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
    pub fn new(repo: &Repository, base_ref: impl Into<String>) -> Result<Self, git2::Error> {
        let base_ref = base_ref.into();
        let base = {
            let base_commit = repo.revparse_single(&base_ref)?.peel_to_commit()?;
            base_commit.id()
        };

        Ok(Self {
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

    pub fn try_append(
        &mut self,
        repo: &Repository,
        patch_id: PatchId,
        commit: Oid,
        summary: Option<String>,
        author: SignatureInfo,
        committer: SignatureInfo,
    ) -> Result<(), MergeQueueError> {
        let patch_commit = repo.find_commit(commit)?;
        let author_signature = author.to_signature()?;
        let committer_signature = committer.to_signature()?;
        let new_tip = Self::cherry_pick_patch(
            repo,
            self.tip,
            summary.as_deref(),
            &author_signature,
            &committer_signature,
            &patch_commit,
        )?;

        let patch = PatchEntry {
            patch_id,
            commit,
            queued_commit: new_tip,
            summary,
            author,
            committer,
        };
        self.tip = new_tip;
        self.patches.push(patch);

        Ok(())
    }

    pub fn try_squash_append_diff(
        &mut self,
        repo: &Repository,
        patch_id: PatchId,
        diff: &str,
        summary: Option<&str>,
    ) -> Result<(), MergeQueueError> {
        let tip_commit = repo.find_commit(self.tip)?;
        let tip_tree = tip_commit.tree()?;
        let git_diff = Diff::from_buffer(diff.as_bytes())?;
        let mut index = match repo.apply_to_tree(&tip_tree, &git_diff, None) {
            Ok(index) => index,
            Err(err) => {
                if err.class() == git2::ErrorClass::Patch {
                    return Err(MergeQueueError::Unmergeable(tip_commit.id()));
                }
                return Err(err.into());
            }
        };
        if index.has_conflicts() {
            return Err(MergeQueueError::Unmergeable(tip_commit.id()));
        }

        let tree_oid = index.write_tree_to(repo)?;
        let tree = repo.find_tree(tree_oid)?;
        let default_signature =
            Signature::now("metis", "metis@localhost").map_err(MergeQueueError::Git)?;
        let author_signature = repo.signature().unwrap_or(default_signature.clone());
        let committer_signature = repo.signature().unwrap_or(default_signature);
        let message = summary
            .filter(|value| !value.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Apply patch {patch_id}"));

        let squashed_commit = repo.commit(
            None,
            &author_signature,
            &committer_signature,
            &message,
            &tree,
            &[&tip_commit],
        )?;

        let author = SignatureInfo::from_signature(&author_signature);
        let committer = SignatureInfo::from_signature(&committer_signature);

        self.try_append(
            repo,
            patch_id,
            squashed_commit,
            Some(message),
            author,
            committer,
        )
    }

    pub fn evict(
        &mut self,
        repo: &Repository,
        commit_id: Oid,
    ) -> Result<Vec<PatchEntry>, MergeQueueError> {
        if !self.patches.iter().any(|entry| entry.commit == commit_id) {
            return Ok(Vec::new());
        }

        let existing = std::mem::take(&mut self.patches);
        let mut kept = Vec::new();
        let mut evicted = Vec::new();
        let mut current_tip = self.base;

        let mut existing_iter = existing.into_iter().peekable();

        for patch in existing_iter.by_ref() {
            if patch.commit == commit_id {
                evicted.push(patch);
                break;
            }

            current_tip = patch.queued_commit;
            kept.push(patch);
        }

        for mut patch in existing_iter {
            let patch_commit = repo.find_commit(patch.commit)?;
            let author_signature = patch.author.to_signature()?;
            let committer_signature = patch.committer.to_signature()?;
            match Self::cherry_pick_patch(
                repo,
                current_tip,
                patch.summary.as_deref(),
                &author_signature,
                &committer_signature,
                &patch_commit,
            ) {
                Ok(new_tip) => {
                    patch.queued_commit = new_tip;
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

    fn cherry_pick_patch(
        repo: &Repository,
        current_tip: Oid,
        summary: Option<&str>,
        author: &Signature<'_>,
        committer: &Signature<'_>,
        patch_commit: &Commit<'_>,
    ) -> Result<Oid, MergeQueueError> {
        let tip_commit = repo.find_commit(current_tip)?;
        let merge_options = MergeOptions::new();
        let mut index =
            repo.cherrypick_commit(patch_commit, &tip_commit, 0, Some(&merge_options))?;

        if index.has_conflicts() {
            return Err(MergeQueueError::Unmergeable(patch_commit.id()));
        }

        let tree_oid = index.write_tree_to(repo)?;
        let tree = repo.find_tree(tree_oid)?;
        let message = summary
            .map(str::to_owned)
            .or_else(|| patch_commit.summary().map(str::to_owned))
            .unwrap_or_else(|| format!("Cherry-pick {} onto {}", patch_commit.id(), current_tip));

        let new_tip = repo.commit(None, author, committer, &message, &tree, &[&tip_commit])?;

        Ok(new_tip)
    }
}

#[cfg(test)]
mod tests {
    use super::{MergeQueueError, MergeQueueImpl, SignatureInfo};
    use anyhow::Result;
    use git2::{Oid, Repository, ResetType, Signature};
    use metis_common::PatchId;
    use std::{fs, path::Path};
    use tempfile::TempDir;

    const BASE_REF: &str = "refs/heads/base";

    struct ScriptedRepo {
        tempdir: TempDir,
        repo: Repository,
        initial: Oid,
    }

    impl ScriptedRepo {
        fn new(initial_spec: &str) -> Result<Self> {
            let tempdir = TempDir::new()?;
            let repo = Repository::init(tempdir.path())?;
            let initial = initial_commit(&repo, initial_spec)?;

            Ok(Self {
                tempdir,
                repo,
                initial,
            })
        }

        fn base_from<S: AsRef<str>>(
            &self,
            changes: impl IntoIterator<Item = S>,
        ) -> Result<(Oid, Vec<Oid>)> {
            let history = commit_script(&self.repo, self.initial, changes)?;
            let base = history.last().copied().unwrap_or(self.initial);
            self.repo.reference(BASE_REF, base, true, "set base ref")?;

            Ok((base, history))
        }

        fn commit_chain<S: AsRef<str>>(
            &self,
            parent: Oid,
            changes: impl IntoIterator<Item = S>,
        ) -> Result<Vec<Oid>> {
            commit_script(&self.repo, parent, changes)
        }

        fn queue_repo(&self) -> Result<Repository> {
            Ok(Repository::open(self.tempdir.path())?)
        }

        fn repo(&self) -> &Repository {
            &self.repo
        }
    }

    #[test]
    fn try_append_advances_tip_and_records_patches() -> Result<()> {
        let scripted = ScriptedRepo::new("file.txt:initial\n")?;
        let (base_commit, _) =
            scripted.base_from(["file.txt:base\n", "file.txt:base updated\n"])?;

        let patches = scripted.commit_chain(
            base_commit,
            ["file.txt:from patch1\n", "file.txt:from patch2\n"],
        )?;
        let patch1 = patches[0];
        let patch2 = patches[1];

        let repo = scripted.queue_repo()?;
        let mut queue = MergeQueueImpl::new(&repo, BASE_REF)?;
        let patch1_id = PatchId::new();
        let patch2_id = PatchId::new();

        let (author1, committer1) = commit_signatures(scripted.repo(), patch1)?;
        queue.try_append(
            &repo,
            patch1_id.clone(),
            patch1,
            Some("first patch".to_string()),
            author1,
            committer1,
        )?;
        let (author2, committer2) = commit_signatures(scripted.repo(), patch2)?;
        queue.try_append(&repo, patch2_id.clone(), patch2, None, author2, committer2)?;

        assert_eq!(queue.patches().len(), 2);
        assert!(
            queue
                .patches()
                .iter()
                .all(|entry| entry.queued_commit != Oid::zero())
        );
        assert_eq!(
            queue
                .patches()
                .iter()
                .map(|entry| entry.patch_id.clone())
                .collect::<Vec<_>>(),
            vec![patch1_id, patch2_id]
        );
        assert_eq!(queue.base(), base_commit);
        assert_ne!(queue.tip(), queue.base());
        assert_eq!(
            queue.tip(),
            queue
                .patches()
                .last()
                .map(|patch| patch.queued_commit)
                .unwrap()
        );
        assert_eq!(
            file_at_commit(scripted.repo(), queue.tip(), "file.txt")?,
            "from patch2\n"
        );

        Ok(())
    }

    #[test]
    fn try_append_rejects_unmergeable_patch() -> Result<()> {
        let scripted = ScriptedRepo::new("file.txt:initial\n")?;
        let (base_commit, _) =
            scripted.base_from(["file.txt:base\n", "file.txt:base updated\n"])?;

        let patch1 = scripted
            .commit_chain(base_commit, ["file.txt:left change\n"])?
            .pop()
            .unwrap();
        let patch2 = scripted
            .commit_chain(base_commit, ["file.txt:right change\n"])?
            .pop()
            .unwrap();

        let repo = scripted.queue_repo()?;
        let mut queue = MergeQueueImpl::new(&repo, BASE_REF)?;
        let patch1_id = PatchId::new();
        let patch2_id = PatchId::new();

        let (author1, committer1) = commit_signatures(scripted.repo(), patch1)?;
        queue.try_append(&repo, patch1_id, patch1, None, author1, committer1)?;
        let tip_after_first = queue.tip();

        let (author2, committer2) = commit_signatures(scripted.repo(), patch2)?;
        let result = queue.try_append(
            &repo,
            patch2_id,
            patch2,
            Some("conflicting patch".to_string()),
            author2,
            committer2,
        );

        assert!(matches!(
            result,
            Err(MergeQueueError::Unmergeable(id)) if id == patch2
        ));
        assert_eq!(queue.tip(), tip_after_first);
        assert_eq!(queue.patches().len(), 1);

        Ok(())
    }

    #[test]
    fn try_squash_append_squashes_range_into_queue() -> Result<()> {
        let scripted = ScriptedRepo::new("file.txt:initial\n")?;
        let (base, _) = scripted.base_from(["file.txt:base\n"])?;

        let commits = scripted.commit_chain(
            base,
            ["file.txt:feature start\n", "file.txt:feature refined\n"],
        )?;
        let head = *commits.last().expect("commit chain non-empty");
        let repo_path = scripted
            .repo()
            .workdir()
            .expect("scripted repo should have a workdir");
        let diff = diff_for_commits(repo_path, base, head)?;

        let repo = scripted.queue_repo()?;
        let mut queue = MergeQueueImpl::new(&repo, BASE_REF)?;

        let patch_id = PatchId::new();
        queue.try_squash_append_diff(&repo, patch_id, &diff, Some("feature refined"))?;

        assert_eq!(queue.patches().len(), 1);
        assert_eq!(
            file_at_commit(scripted.repo(), queue.tip(), "file.txt")?,
            "feature refined\n"
        );

        Ok(())
    }

    #[test]
    fn try_append_handles_divergent_branches_with_additional_files() -> Result<()> {
        let scripted = ScriptedRepo::new("main.txt:initial\n")?;
        let (base, base_history) = scripted.base_from([
            "main.txt:base v1\n",
            "main.txt:base v2\n",
            "main.txt:base v3\n",
        ])?;
        let feature_commits = scripted.commit_chain(
            base_history[0],
            [
                "feature.txt:branch feature start\n",
                "feature.txt:branch feature extended\n",
            ],
        )?;
        let hotfix = scripted.commit_chain(base, ["main.txt:base v3 hotfix\n"])?;

        let repo = scripted.queue_repo()?;
        let mut queue = MergeQueueImpl::new(&repo, BASE_REF)?;
        let feature_patch_ids = [PatchId::new(), PatchId::new()];
        let hotfix_patch_id = PatchId::new();

        let (author1, committer1) = commit_signatures(scripted.repo(), feature_commits[0])?;
        queue.try_append(
            &repo,
            feature_patch_ids[0].clone(),
            feature_commits[0],
            Some("feature kickoff".to_string()),
            author1,
            committer1,
        )?;
        let (author2, committer2) = commit_signatures(scripted.repo(), feature_commits[1])?;
        queue.try_append(
            &repo,
            feature_patch_ids[1].clone(),
            feature_commits[1],
            Some("feature refinement".to_string()),
            author2,
            committer2,
        )?;
        let (author3, committer3) = commit_signatures(scripted.repo(), hotfix[0])?;
        queue.try_append(
            &repo,
            hotfix_patch_id,
            hotfix[0],
            Some("stability hotfix".to_string()),
            author3,
            committer3,
        )?;

        assert_eq!(
            file_at_commit(scripted.repo(), queue.tip(), "main.txt")?,
            "base v3 hotfix\n"
        );
        assert_eq!(
            file_at_commit(scripted.repo(), queue.tip(), "feature.txt")?,
            "branch feature extended\n"
        );

        Ok(())
    }

    #[test]
    fn evict_removes_patch_and_reapplies_queue() -> Result<()> {
        let scripted = ScriptedRepo::new("file.txt:initial\n")?;
        let (base, _) = scripted.base_from(["file.txt:base branch\n"])?;

        let patch1 = scripted
            .commit_chain(base, ["file.txt:aligned value\n"])?
            .pop()
            .unwrap();
        let patch2 = scripted
            .commit_chain(scripted.initial, ["file.txt:aligned value\n"])?
            .pop()
            .unwrap();

        let repo = scripted.queue_repo()?;
        let mut queue = MergeQueueImpl::new(&repo, BASE_REF)?;
        let patch1_id = PatchId::new();
        let patch2_id = PatchId::new();

        let (author1, committer1) = commit_signatures(scripted.repo(), patch1)?;
        queue.try_append(
            &repo,
            patch1_id.clone(),
            patch1,
            Some("kept patch".to_string()),
            author1,
            committer1,
        )?;
        let (author2, committer2) = commit_signatures(scripted.repo(), patch2)?;
        queue.try_append(
            &repo,
            patch2_id.clone(),
            patch2,
            Some("dependent patch".to_string()),
            author2,
            committer2,
        )?;

        let evicted = queue.evict(&repo, patch1)?;

        assert_eq!(evicted.len(), 2);
        assert!(evicted.iter().any(|entry| entry.commit == patch1));
        assert!(evicted.iter().any(|entry| entry.commit == patch2));
        assert!(evicted.iter().any(|entry| entry.patch_id == patch1_id));
        assert!(evicted.iter().any(|entry| entry.patch_id == patch2_id));
        assert_eq!(queue.patches().len(), 0);
        assert_eq!(queue.tip(), base);

        Ok(())
    }

    #[test]
    fn evict_replays_dependent_branch_and_preserves_predecessors() -> Result<()> {
        let scripted = ScriptedRepo::new("file.txt:initial\n")?;
        let (base, _) = scripted.base_from(["file.txt:base v1\n", "file.txt:base v2\n"])?;

        let feature_patch = scripted
            .commit_chain(base, ["feature.txt:queue keeps earlier patches\n"])?
            .pop()
            .unwrap();
        let hotfix = scripted
            .commit_chain(base, ["file.txt:base v2 hotfix\n"])?
            .pop()
            .unwrap();
        let hotfix_follow_up = scripted
            .commit_chain(hotfix, ["file.txt:base v2 hotfix follow up\n"])?
            .pop()
            .unwrap();

        let repo = scripted.queue_repo()?;
        let mut queue = MergeQueueImpl::new(&repo, BASE_REF)?;
        let feature_patch_id = PatchId::new();
        let hotfix_patch_id = PatchId::new();
        let follow_up_patch_id = PatchId::new();

        let (feature_author, feature_committer) =
            commit_signatures(scripted.repo(), feature_patch)?;
        queue.try_append(
            &repo,
            feature_patch_id.clone(),
            feature_patch,
            Some("early queue patch".to_string()),
            feature_author,
            feature_committer,
        )?;
        let (hotfix_author, hotfix_committer) = commit_signatures(scripted.repo(), hotfix)?;
        queue.try_append(
            &repo,
            hotfix_patch_id.clone(),
            hotfix,
            Some("middle hotfix".to_string()),
            hotfix_author,
            hotfix_committer,
        )?;
        let (follow_author, follow_committer) =
            commit_signatures(scripted.repo(), hotfix_follow_up)?;
        queue.try_append(
            &repo,
            follow_up_patch_id.clone(),
            hotfix_follow_up,
            Some("dependent follow up".to_string()),
            follow_author,
            follow_committer,
        )?;

        let evicted = queue.evict(&repo, hotfix)?;

        assert_eq!(evicted.len(), 2);
        assert!(evicted.iter().any(|entry| entry.commit == hotfix));
        assert!(evicted.iter().any(|entry| entry.commit == hotfix_follow_up));
        assert!(
            evicted
                .iter()
                .any(|entry| entry.patch_id == hotfix_patch_id)
        );
        assert!(
            evicted
                .iter()
                .any(|entry| entry.patch_id == follow_up_patch_id)
        );
        assert_eq!(queue.patches().len(), 1);
        assert_eq!(queue.patches()[0].commit, feature_patch);
        assert_eq!(queue.patches()[0].patch_id, feature_patch_id);
        assert_eq!(
            file_at_commit(scripted.repo(), queue.tip(), "feature.txt")?,
            "queue keeps earlier patches\n"
        );
        assert_eq!(
            file_at_commit(scripted.repo(), queue.tip(), "file.txt")?,
            "base v2\n"
        );

        Ok(())
    }

    fn diff_for_commits(repo_path: &Path, base: Oid, head: Oid) -> Result<String> {
        let repo_str = repo_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("invalid repo path"))?;
        let output = std::process::Command::new("git")
            .args(["-C", repo_str, "diff", &format!("{base}..{head}")])
            .output()?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).to_string());
        }

        anyhow::bail!(
            "git diff failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    }

    fn commit_script<S: AsRef<str>>(
        repo: &Repository,
        parent: Oid,
        changes: impl IntoIterator<Item = S>,
    ) -> Result<Vec<Oid>> {
        let mut current = parent;
        let mut commits = Vec::new();
        for change in changes.into_iter() {
            current = commit_with_parent(repo, current, change.as_ref())?;
            commits.push(current);
        }
        Ok(commits)
    }

    fn initial_commit(repo: &Repository, change: &str) -> Result<Oid> {
        let (path, contents) = parse_change_spec(change)?;
        let mut index = repo.index()?;
        write_file(repo, path, contents)?;
        index.add_path(Path::new(path))?;
        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        let signature = test_signature()?;

        repo.commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[])
            .map_err(Into::into)
    }

    fn commit_with_parent(repo: &Repository, parent: Oid, change: &str) -> Result<Oid> {
        let (path, contents) = parse_change_spec(change)?;
        repo.reset(&repo.find_object(parent, None)?, ResetType::Hard, None)?;
        write_file(repo, path, contents)?;
        let mut index = repo.index()?;
        index.add_path(Path::new(path))?;
        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        let signature = test_signature()?;
        let parent_commit = repo.find_commit(parent)?;

        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            change,
            &tree,
            &[&parent_commit],
        )
        .map_err(Into::into)
    }

    fn parse_change_spec(change: &str) -> Result<(&str, &str)> {
        let (path, contents) = change
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("commit spec must be path:contents"))?;
        let path = path.trim();
        if path.is_empty() {
            anyhow::bail!("commit spec must include a file path");
        }
        Ok((path, contents))
    }

    fn write_file(repo: &Repository, path: &str, contents: &str) -> Result<()> {
        let path = repo
            .workdir()
            .map(|dir| dir.join(path))
            .ok_or_else(|| anyhow::anyhow!("repository is bare"))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, contents)?;
        Ok(())
    }

    fn file_at_commit(repo: &Repository, commit: Oid, path: &str) -> Result<String> {
        let commit = repo.find_commit(commit)?;
        let tree = commit.tree()?;
        let entry = tree.get_path(Path::new(path))?;
        let blob = repo.find_blob(entry.id())?;

        Ok(String::from_utf8_lossy(blob.content()).into_owned())
    }

    fn commit_signatures(repo: &Repository, commit: Oid) -> Result<(SignatureInfo, SignatureInfo)> {
        let commit = repo.find_commit(commit)?;
        Ok((
            SignatureInfo::from_signature(&commit.author()),
            SignatureInfo::from_signature(&commit.committer()),
        ))
    }

    fn test_signature() -> Result<Signature<'static>> {
        Signature::now("metis", "metis@example.com").map_err(Into::into)
    }
}
