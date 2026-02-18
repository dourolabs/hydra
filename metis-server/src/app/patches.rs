use crate::{
    domain::{actors::ActorRef, patches::Patch},
    store::{ReadOnlyStore, Status, StoreError},
};
use metis_common::{
    PatchId, TaskId, VersionNumber, Versioned, api::v1 as api, api::v1::patches::SearchPatchesQuery,
};
use thiserror::Error;

use super::app_state::AppState;
use super::issues::UpsertIssueError;

#[derive(Debug, Error)]
pub enum UpsertPatchError {
    #[error("job '{job_id}' not found")]
    JobNotFound {
        #[source]
        source: StoreError,
        job_id: TaskId,
    },
    #[error("failed to validate job status for '{job_id}'")]
    JobStatusLookup {
        #[source]
        source: StoreError,
        job_id: TaskId,
    },
    #[error("created_by must reference a running job")]
    JobNotRunning {
        job_id: TaskId,
        status: Option<Status>,
    },
    #[error("patch '{patch_id}' not found")]
    PatchNotFound {
        #[source]
        source: StoreError,
        patch_id: PatchId,
    },
    #[error("patch store operation failed")]
    Store {
        #[source]
        source: StoreError,
    },
    #[error("failed to load merge-request issues for patch '{patch_id}'")]
    MergeRequestLookup {
        #[source]
        source: StoreError,
        patch_id: PatchId,
    },
    #[error("failed to create merge-request issue for patch '{patch_id}'")]
    MergeRequestCreate {
        #[source]
        source: UpsertIssueError,
        patch_id: PatchId,
    },
    #[error("failed to update merge-request issue '{issue_id}' for patch '{patch_id}'")]
    MergeRequestUpdate {
        #[source]
        source: StoreError,
        patch_id: PatchId,
        issue_id: metis_common::issues::IssueId,
    },
    #[error("an open patch '{existing_patch_id}' already exists for branch '{branch_name}'")]
    DuplicateBranchName {
        existing_patch_id: PatchId,
        branch_name: String,
    },
    #[error("{0}")]
    PolicyViolation(#[from] crate::policy::PolicyViolation),
}

impl AppState {
    pub async fn get_patch(
        &self,
        patch_id: &PatchId,
        include_deleted: bool,
    ) -> Result<Versioned<Patch>, StoreError> {
        let store = self.store.as_ref();
        store.get_patch(patch_id, include_deleted).await
    }

    pub async fn get_patch_versions(
        &self,
        patch_id: &PatchId,
    ) -> Result<Vec<Versioned<Patch>>, StoreError> {
        let store = self.store.as_ref();
        store.get_patch_versions(patch_id).await
    }

    pub async fn list_patches(&self) -> Result<Vec<(PatchId, Versioned<Patch>)>, StoreError> {
        let store = self.store.as_ref();
        store.list_patches(&SearchPatchesQuery::default()).await
    }

    pub async fn list_patches_with_query(
        &self,
        query: &SearchPatchesQuery,
    ) -> Result<Vec<(PatchId, Versioned<Patch>)>, StoreError> {
        let store = self.store.as_ref();
        store.list_patches(query).await
    }

    pub async fn delete_patch(
        &self,
        patch_id: &PatchId,
        actor: ActorRef,
    ) -> Result<(), StoreError> {
        self.store.delete_patch_with_actor(patch_id, actor).await?;
        Ok(())
    }

    pub async fn upsert_patch(
        &self,
        actor: ActorRef,
        patch_id: Option<PatchId>,
        request: api::patches::UpsertPatchRequest,
    ) -> Result<(PatchId, VersionNumber), UpsertPatchError> {
        let api::patches::UpsertPatchRequest { patch, .. } = request;
        let mut patch: Patch = patch.into();

        let store = self.store.as_ref();
        let (patch_id, version) = match patch_id {
            Some(id) => {
                let existing_patch =
                    store
                        .get_patch(&id, false)
                        .await
                        .map_err(|source| match source {
                            StoreError::PatchNotFound(_) => UpsertPatchError::PatchNotFound {
                                patch_id: id.clone(),
                                source,
                            },
                            other => UpsertPatchError::Store { source: other },
                        })?;

                patch.created_by = existing_patch.item.created_by;
                if patch.github.is_none() {
                    patch.github = existing_patch.item.github.clone();
                }

                let version = self
                    .store
                    .update_patch_with_actor(&id, patch, actor.clone())
                    .await
                    .map_err(|source| match source {
                        StoreError::PatchNotFound(_) => UpsertPatchError::PatchNotFound {
                            patch_id: id.clone(),
                            source,
                        },
                        other => UpsertPatchError::Store { source: other },
                    })?;

                (id, version)
            }
            None => {
                // Run restriction policies before persisting
                {
                    self.policy_engine
                        .check_create_patch(&patch, store, &actor)
                        .await?;
                }

                let (id, version) = self
                    .store
                    .add_patch_with_actor(patch, actor)
                    .await
                    .map_err(|source| match source {
                        StoreError::PatchNotFound(id) => UpsertPatchError::PatchNotFound {
                            patch_id: id.clone(),
                            source: StoreError::PatchNotFound(id),
                        },
                        other => UpsertPatchError::Store { source: other },
                    })?;
                (id, version)
            }
        };

        tracing::info!(patch_id = %patch_id, "patch stored successfully");

        Ok((patch_id, version))
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        app::test_helpers::{
            github_pull_request_response, poll_until, sample_task, start_test_automation_runner,
        },
        domain::{
            actors::{Actor, ActorRef},
            patches::{GithubPr, Patch, PatchStatus},
            users::{User, Username},
        },
        store::Status,
        test_utils::{
            add_repository, github_user_response, test_state_handles,
            test_state_with_github_api_base_url,
        },
    };
    use chrono::Utc;
    use httpmock::Method::PATCH;
    use httpmock::prelude::*;
    use metis_common::{RepoName, TaskId, api::v1 as api};

    #[tokio::test]
    async fn upsert_patch_sync_github_updates_existing_pr() -> anyhow::Result<()> {
        let github_server = MockServer::start_async().await;
        let user_mock = github_server.mock(|when, then| {
            when.method(GET).path("/user");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(github_user_response("octo", 42));
        });
        let update_mock = github_server.mock(|when, then| {
            when.method(PATCH)
                .path("/repos/octo/repo/pulls/42")
                .json_body_partial(r#"{"title":"Updated title","body":"Updated description"}"#);
            then.status(200)
                .header("content-type", "application/json")
                .json_body(github_pull_request_response(
                    42,
                    "feature",
                    "main",
                    "https://example.com/pr/42",
                ));
        });

        let handles = test_state_with_github_api_base_url(github_server.base_url());
        let runner = start_test_automation_runner(&handles.state);
        let username = Username::from("octo");
        let user = User::new(
            username.clone(),
            42,
            "token-123".to_string(),
            "refresh-123".to_string(),
        );
        handles
            .store
            .as_ref()
            .add_user(user, &ActorRef::test())
            .await?;
        let (actor, _auth_token) = Actor::new_for_user(username);
        handles
            .store
            .as_ref()
            .add_actor(actor.clone(), &ActorRef::test())
            .await?;
        let repo_name = RepoName::new("octo", "repo")?;
        let existing_patch = Patch::new(
            "Original".to_string(),
            "Original description".to_string(),
            "diff".to_string(),
            PatchStatus::Open,
            false,
            Some(TaskId::new()),
            Username::from("test-creator"),
            Vec::new(),
            repo_name.clone(),
            Some(GithubPr::new(
                "octo".to_string(),
                "repo".to_string(),
                42,
                Some("old-head".to_string()),
                Some("old-base".to_string()),
                None,
                None,
            )),
        );

        let (patch_id, _) = handles
            .store
            .as_ref()
            .add_patch(existing_patch, &ActorRef::test())
            .await?;

        let mut request_patch = Patch::new(
            "Updated title".to_string(),
            "Updated description".to_string(),
            "diff".to_string(),
            PatchStatus::Open,
            false,
            None,
            Username::from("test-creator"),
            Vec::new(),
            repo_name,
            None,
        );
        request_patch.branch_name = Some("feature".to_string());
        let request = api::patches::UpsertPatchRequest::new(request_patch.into());

        handles
            .state
            .upsert_patch(ActorRef::from(&actor), Some(patch_id.clone()), request)
            .await?;

        // Poll until the automation updates the github metadata.
        let github = poll_until(std::time::Duration::from_secs(5), || {
            let store = handles.store.clone();
            let pid = patch_id.clone();
            async move {
                let p = store.as_ref().get_patch(&pid, false).await.ok()?;
                let gh = p.item.github?;
                if gh.head_ref.as_deref() == Some("feature") {
                    Some(gh)
                } else {
                    None
                }
            }
        })
        .await
        .expect("github metadata should be updated by automation");

        assert_eq!(github.number, 42);
        assert_eq!(github.owner, "octo");
        assert_eq!(github.repo, "repo");
        assert_eq!(github.head_ref.as_deref(), Some("feature"));
        assert_eq!(github.base_ref.as_deref(), Some("main"));
        assert_eq!(github.url.as_deref(), Some("https://example.com/pr/42"));

        user_mock.assert_async().await;
        update_mock.assert_async().await;

        runner.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn upsert_patch_sync_github_creates_pr_and_persists_github() -> anyhow::Result<()> {
        let github_server = MockServer::start_async().await;
        let user_mock = github_server.mock(|when, then| {
            when.method(GET).path("/user");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(github_user_response("octo", 42));
        });
        let create_mock = github_server.mock(|when, then| {
            when.method(POST)
                .path("/repos/octo/repo/pulls")
                .json_body_partial(
                    r#"{"title":"New patch","head":"metis-t-test","base":"main","body":"New patch description"}"#,
                );
            then.status(200)
                .header("content-type", "application/json")
                .json_body(github_pull_request_response(
                    99,
                    "metis-t-test",
                    "main",
                    "https://example.com/pr/99",
                ));
        });

        let handles = test_state_with_github_api_base_url(github_server.base_url());
        let runner = start_test_automation_runner(&handles.state);
        let username = Username::from("octo");
        let user = User::new(
            username.clone(),
            42,
            "token-456".to_string(),
            "refresh-456".to_string(),
        );
        handles
            .store
            .as_ref()
            .add_user(user, &ActorRef::test())
            .await?;
        let (actor, _auth_token) = Actor::new_for_user(username);
        handles
            .store
            .as_ref()
            .add_actor(actor.clone(), &ActorRef::test())
            .await?;
        let repo_name = RepoName::new("octo", "repo")?;
        add_repository(
            &handles.state,
            repo_name.clone(),
            crate::app::Repository::new(
                "https://example.com/repo.git".to_string(),
                Some("main".to_string()),
                None,
            ),
        )
        .await?;

        let mut task = sample_task();
        let created_at = Utc::now();
        let (task_id, _) = handles
            .store
            .as_ref()
            .add_task(task.clone(), created_at, &ActorRef::test())
            .await?;
        task.status = Status::Running;
        handles
            .store
            .as_ref()
            .update_task(&task_id, task, &ActorRef::test())
            .await?;
        let mut patch = Patch::new(
            "New patch".to_string(),
            "New patch description".to_string(),
            "diff".to_string(),
            PatchStatus::Open,
            false,
            Some(task_id),
            Username::from("test-creator"),
            Vec::new(),
            repo_name,
            None,
        );
        patch.branch_name = Some("metis-t-test".to_string());
        let request = api::patches::UpsertPatchRequest::new(patch.into());

        let (patch_id, _) = handles
            .state
            .upsert_patch(ActorRef::from(&actor), None, request)
            .await?;

        // Poll until the automation creates the github metadata.
        let github = poll_until(std::time::Duration::from_secs(5), || {
            let store = handles.store.clone();
            let pid = patch_id.clone();
            async move {
                let p = store.as_ref().get_patch(&pid, false).await.ok()?;
                p.item.github
            }
        })
        .await
        .expect("github metadata should be created by automation");

        assert_eq!(github.number, 99);
        assert_eq!(github.owner, "octo");
        assert_eq!(github.repo, "repo");
        assert_eq!(github.head_ref.as_deref(), Some("metis-t-test"));
        assert_eq!(github.base_ref.as_deref(), Some("main"));
        assert_eq!(github.url.as_deref(), Some("https://example.com/pr/99"));

        user_mock.assert_async().await;
        create_mock.assert_async().await;

        runner.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn upsert_patch_allows_same_branch_after_close() -> anyhow::Result<()> {
        let handles = test_state_handles();
        let repo_name = RepoName::new("octo", "repo")?;

        let mut patch1 = Patch::new(
            "First patch".to_string(),
            "desc".to_string(),
            "diff".to_string(),
            PatchStatus::Open,
            false,
            None,
            Username::from("test-creator"),
            Vec::new(),
            repo_name.clone(),
            None,
        );
        patch1.branch_name = Some("feature/foo".to_string());
        let request1 = api::patches::UpsertPatchRequest::new(patch1.into());
        let (patch1_id, _) = handles
            .state
            .upsert_patch(ActorRef::test(), None, request1)
            .await?;

        // Close the first patch
        let mut closed_patch = handles.store.get_patch(&patch1_id, false).await?.item;
        closed_patch.status = PatchStatus::Closed;
        handles
            .store
            .update_patch(&patch1_id, closed_patch, &ActorRef::test())
            .await?;

        // Creating a new patch with the same branch_name should succeed
        let mut patch2 = Patch::new(
            "Second patch".to_string(),
            "desc".to_string(),
            "diff".to_string(),
            PatchStatus::Open,
            false,
            None,
            Username::from("test-creator"),
            Vec::new(),
            repo_name,
            None,
        );
        patch2.branch_name = Some("feature/foo".to_string());
        let request2 = api::patches::UpsertPatchRequest::new(patch2.into());
        handles
            .state
            .upsert_patch(ActorRef::test(), None, request2)
            .await?;

        Ok(())
    }

    #[tokio::test]
    async fn upsert_patch_update_allows_same_branch() -> anyhow::Result<()> {
        let handles = test_state_handles();
        let repo_name = RepoName::new("octo", "repo")?;

        let mut patch1 = Patch::new(
            "First patch".to_string(),
            "desc".to_string(),
            "diff".to_string(),
            PatchStatus::Open,
            false,
            None,
            Username::from("test-creator"),
            Vec::new(),
            repo_name.clone(),
            None,
        );
        patch1.branch_name = Some("feature/foo".to_string());
        let request1 = api::patches::UpsertPatchRequest::new(patch1.into());
        let (patch1_id, _) = handles
            .state
            .upsert_patch(ActorRef::test(), None, request1)
            .await?;

        // Updating the same patch should succeed (the uniqueness check is only
        // on creates, not updates).
        let mut update_patch = Patch::new(
            "Updated title".to_string(),
            "desc".to_string(),
            "diff".to_string(),
            PatchStatus::Open,
            false,
            None,
            Username::from("test-creator"),
            Vec::new(),
            repo_name,
            None,
        );
        update_patch.branch_name = Some("feature/foo".to_string());
        let request2 = api::patches::UpsertPatchRequest::new(update_patch.into());
        handles
            .state
            .upsert_patch(ActorRef::test(), Some(patch1_id), request2)
            .await?;

        Ok(())
    }

    #[tokio::test]
    async fn upsert_patch_preserves_creator_set_by_caller() -> anyhow::Result<()> {
        let handles = test_state_handles();
        let repo_name = RepoName::new("octo", "repo")?;

        let creator_username = Username::from("the-human");
        let task_id = TaskId::new();
        let (actor, _auth_token) = Actor::new_for_task(task_id.clone(), creator_username.clone());
        handles
            .store
            .as_ref()
            .add_actor(actor.clone(), &ActorRef::test())
            .await?;

        let patch = Patch::new(
            "Agent patch".to_string(),
            "Created by an agent".to_string(),
            "diff content".to_string(),
            PatchStatus::Open,
            false,
            None,
            creator_username.clone(),
            Vec::new(),
            repo_name,
            None,
        );
        let request = api::patches::UpsertPatchRequest::new(patch.into());

        let (patch_id, _) = handles
            .state
            .upsert_patch(ActorRef::from(&actor), None, request)
            .await?;

        let stored = handles.store.as_ref().get_patch(&patch_id, false).await?;
        assert_eq!(
            stored.item.creator, creator_username,
            "patch.creator should be preserved as set by the caller"
        );

        Ok(())
    }
}
