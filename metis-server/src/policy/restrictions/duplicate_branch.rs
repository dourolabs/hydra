use async_trait::async_trait;
use metis_common::api::v1::patches::{PatchStatus as ApiPatchStatus, SearchPatchesQuery};

use crate::policy::context::{Operation, OperationPayload, RestrictionContext};
use crate::policy::{PolicyViolation, Restriction};

/// Rejects patches with branch names already used by open patches.
#[derive(Default)]
pub struct DuplicateBranchRestriction;

impl DuplicateBranchRestriction {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Restriction for DuplicateBranchRestriction {
    fn name(&self) -> &str {
        "duplicate_branch_name"
    }

    async fn evaluate(&self, ctx: &RestrictionContext<'_>) -> Result<(), PolicyViolation> {
        // Only applies to patch creation
        if ctx.operation != Operation::CreatePatch {
            return Ok(());
        }

        let OperationPayload::Patch { new, .. } = ctx.payload else {
            return Ok(());
        };

        let Some(ref branch_name) = new.branch_name else {
            return Ok(());
        };

        let query = SearchPatchesQuery::new(None, None)
            .with_status(vec![ApiPatchStatus::Open, ApiPatchStatus::ChangesRequested])
            .with_branch_name(branch_name.clone());

        let existing = ctx
            .store
            .list_patches(&query)
            .await
            .map_err(|e| PolicyViolation {
                policy_name: self.name().to_string(),
                message: format!("Failed to check for duplicate branches: {e}"),
            })?;

        if let Some((existing_id, _)) = existing.first() {
            return Err(PolicyViolation {
                policy_name: self.name().to_string(),
                message: format!(
                    "Branch name \"{branch_name}\" is already in use by open patch {existing_id}. \
                     Use a different branch name."
                ),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::patches::{Patch, PatchStatus};
    use crate::policy::context::{Operation, OperationPayload, RestrictionContext};
    use crate::store::{MemoryStore, Store};
    use metis_common::RepoName;
    use std::str::FromStr;

    fn make_patch(branch_name: Option<&str>) -> Patch {
        Patch {
            title: "Test patch".to_string(),
            description: String::new(),
            diff: String::new(),
            status: PatchStatus::Open,
            is_automatic_backup: false,
            branch_name: branch_name.map(String::from),
            service_repo_name: RepoName::from_str("test/repo").unwrap(),
            created_by: None,
            github: None,
            commit_range: None,
            reviews: Vec::new(),
            deleted: false,
            sync_github_branch: None,
        }
    }

    #[tokio::test]
    async fn allows_unique_branch_name() {
        let restriction = DuplicateBranchRestriction::new();
        let store = MemoryStore::new();

        let payload = OperationPayload::Patch {
            patch_id: None,
            new: make_patch(Some("feature/new-branch")),
            old: None,
        };
        let ctx = RestrictionContext {
            operation: Operation::CreatePatch,

            repo: None,
            payload: &payload,
            store: &store,
        };
        assert!(restriction.evaluate(&ctx).await.is_ok());
    }

    #[tokio::test]
    async fn rejects_duplicate_branch_name() {
        let restriction = DuplicateBranchRestriction::new();
        let store = MemoryStore::new();

        // Add an existing open patch with the same branch name
        let existing_patch = make_patch(Some("feature/duplicate"));
        store
            .add_patch(existing_patch)
            .await
            .expect("should add patch");

        let payload = OperationPayload::Patch {
            patch_id: None,
            new: make_patch(Some("feature/duplicate")),
            old: None,
        };
        let ctx = RestrictionContext {
            operation: Operation::CreatePatch,

            repo: None,
            payload: &payload,
            store: &store,
        };
        let result = restriction.evaluate(&ctx).await;
        assert!(result.is_err());
        let violation = result.unwrap_err();
        assert_eq!(violation.policy_name, "duplicate_branch_name");
        assert!(violation.message.contains("already in use"));
        assert!(violation.message.contains("feature/duplicate"));
    }

    #[tokio::test]
    async fn allows_patch_without_branch_name() {
        let restriction = DuplicateBranchRestriction::new();
        let store = MemoryStore::new();

        let payload = OperationPayload::Patch {
            patch_id: None,
            new: make_patch(None),
            old: None,
        };
        let ctx = RestrictionContext {
            operation: Operation::CreatePatch,

            repo: None,
            payload: &payload,
            store: &store,
        };
        assert!(restriction.evaluate(&ctx).await.is_ok());
    }
}
