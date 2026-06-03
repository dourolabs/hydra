use crate::{
    domain::{
        actors::{ActorId, ActorRef},
        patches::{Patch, Review},
    },
    store::{ReadOnlyStore, Status, StoreError},
};
use hydra_common::{
    PatchId, SessionId, VersionNumber, Versioned, api::v1 as api,
    api::v1::patches::SearchPatchesQuery, principal::Principal,
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
        job_id: SessionId,
    },
    #[error("failed to validate job status for '{job_id}'")]
    JobStatusLookup {
        #[source]
        source: StoreError,
        job_id: SessionId,
    },
    #[error("actor must reference a running job")]
    JobNotRunning {
        job_id: SessionId,
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
        issue_id: hydra_common::issues::IssueId,
    },
    #[error("an open patch '{existing_patch_id}' already exists for branch '{branch_name}'")]
    DuplicateBranchName {
        existing_patch_id: PatchId,
        branch_name: String,
    },
    /// The authenticated actor is not eligible to author a (newly-submitted)
    /// review on a patch upsert request. Only durable principals
    /// (`Principal::User`, `Principal::Agent`) can be stamped as review
    /// authors; `Adhoc` sessions, `External` actors, `Legacy` identifiers,
    /// and server-internal `System`/`Automation` actors fail here with
    /// HTTP 400.
    #[error("{reason}")]
    InvalidActorForReview {
        actor: Box<ActorRef>,
        reason: String,
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

    pub async fn count_patches(&self, query: &SearchPatchesQuery) -> Result<u64, StoreError> {
        let store = self.store.as_ref();
        store.count_patches(query).await
    }

    pub async fn delete_patch(
        &self,
        patch_id: &PatchId,
        actor: ActorRef,
    ) -> Result<(), StoreError> {
        self.store.delete_patch_with_actor(patch_id, actor).await?;
        Ok(())
    }

    /// Convert + stamp + persist in a single call: the route-handler
    /// entry point for `POST /v1/patches` and `PUT /v1/patches/:id`.
    ///
    /// The embedded review payload (`UpsertReviewRequest`) carries no
    /// author — for each incoming review the server either preserves
    /// the existing stored author (matched against the stored patch by
    /// `(contents, is_approved, submitted_at)`) or stamps the author
    /// from the authenticated `actor`. Server-internal callers (the
    /// GitHub PR poller, etc.) bypass this method and call
    /// [`Self::upsert_patch`] directly with a pre-stamped domain
    /// [`Patch`].
    pub async fn upsert_patch_from_request(
        &self,
        actor: ActorRef,
        patch_id: Option<PatchId>,
        request: api::patches::UpsertPatchRequest,
    ) -> Result<(PatchId, VersionNumber), UpsertPatchError> {
        let patch = self
            .build_patch_from_upsert(&actor, patch_id.as_ref(), request.patch)
            .await?;
        self.upsert_patch(actor, patch_id, patch).await
    }

    /// Persist a fully-constructed domain [`Patch`] — i.e. one with
    /// stamped [`Principal`] review authors already in place.
    /// Server-internal callers (GitHub PR poller) use this method
    /// directly; the HTTP route handlers go through
    /// [`Self::upsert_patch_from_request`].
    pub async fn upsert_patch(
        &self,
        actor: ActorRef,
        patch_id: Option<PatchId>,
        mut patch: Patch,
    ) -> Result<(PatchId, VersionNumber), UpsertPatchError> {
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

    /// Convert a wire-shape [`api::patches::UpsertPatch`] into a
    /// stored-shape domain [`Patch`]. Newly-submitted reviews
    /// (those that do not appear in the prior stored version's
    /// `reviews` array, matched by
    /// `(contents, is_approved, submitted_at)`) are stamped with the
    /// authenticated actor's `Principal`; the rest keep their stored
    /// author. Only [`Principal`]-eligible actors (User, Agent, plus
    /// the legacy `Username` variant) can stamp new reviews — `Adhoc`,
    /// `External`, `Legacy`, and server-internal `System`/`Automation`
    /// actors all return [`UpsertPatchError::InvalidActorForReview`].
    async fn build_patch_from_upsert(
        &self,
        actor: &ActorRef,
        patch_id: Option<&PatchId>,
        upsert: api::patches::UpsertPatch,
    ) -> Result<Patch, UpsertPatchError> {
        let api::patches::UpsertPatch {
            title,
            description,
            diff,
            status,
            is_automatic_backup,
            creator,
            reviews: incoming_reviews,
            service_repo_name,
            github,
            deleted,
            branch_name,
            commit_range,
            base_branch,
            ..
        } = upsert;

        let existing_reviews: Vec<Review> = if let Some(id) = patch_id {
            match self.store.as_ref().get_patch(id, false).await {
                Ok(versioned) => versioned.item.reviews,
                // Treat a missing patch as "no existing reviews"; the
                // subsequent store.update_patch_with_actor will surface
                // the not-found error.
                Err(StoreError::PatchNotFound(_)) => Vec::new(),
                Err(source) => return Err(UpsertPatchError::Store { source }),
            }
        } else {
            Vec::new()
        };

        // Lazily derive the author principal only if there's at least one
        // incoming review without a match in the prior stored version. This
        // keeps "PUT to update non-review fields" working for actor kinds
        // that aren't review-eligible (e.g. legacy Username flows).
        let mut new_review_author: Option<Principal> = None;
        let mut stamped_reviews: Vec<Review> = Vec::with_capacity(incoming_reviews.len());
        for req in incoming_reviews {
            let matched = existing_reviews.iter().find(|existing| {
                existing.contents == req.contents
                    && existing.is_approved == req.is_approved
                    && existing.submitted_at == req.submitted_at
            });
            let author = match matched {
                Some(existing) => existing.author.clone(),
                None => {
                    if new_review_author.is_none() {
                        new_review_author = Some(principal_for_review_author(actor)?);
                    }
                    new_review_author
                        .clone()
                        .expect("new_review_author was just set")
                }
            };
            stamped_reviews.push(Review {
                contents: req.contents,
                is_approved: req.is_approved,
                author,
                submitted_at: req.submitted_at,
            });
        }

        Ok(Patch {
            title,
            description,
            diff,
            status: status.into(),
            is_automatic_backup,
            creator: creator.into(),
            reviews: stamped_reviews,
            service_repo_name,
            github: github.map(Into::into),
            deleted,
            branch_name,
            commit_range: commit_range.map(Into::into),
            base_branch,
        })
    }
}

/// Derive the [`Principal`] to stamp on a newly-submitted review
/// from the authenticated actor. Only durable principals can author
/// reviews.
#[allow(clippy::result_large_err)]
fn principal_for_review_author(actor: &ActorRef) -> Result<Principal, UpsertPatchError> {
    let invalid = |reason: &str| UpsertPatchError::InvalidActorForReview {
        actor: Box::new(actor.clone()),
        reason: reason.to_string(),
    };
    let actor_id = match actor {
        ActorRef::Authenticated { actor_id, .. } => actor_id,
        ActorRef::System { .. } => {
            return Err(invalid(
                "system actor cannot author reviews via the patch upsert API",
            ));
        }
        ActorRef::Automation { .. } => {
            return Err(invalid(
                "automation actor cannot author reviews via the patch upsert API",
            ));
        }
        ActorRef::Trigger { .. } => {
            return Err(invalid(
                "trigger actor cannot author reviews via the patch upsert API",
            ));
        }
    };
    match actor_id {
        ActorId::User(name) => Ok(Principal::User { name: name.clone() }),
        ActorId::Agent(name) => Ok(Principal::Agent { name: name.clone() }),
        ActorId::Adhoc(_) => Err(invalid("ad-hoc sessions cannot author reviews")),
        ActorId::External { .. } => Err(invalid(
            "external actors cannot author reviews via the patch upsert API",
        )),
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        app::test_helpers::{
            github_pull_request_response, poll_until, sample_task, start_test_automation_runner,
        },
        domain::{
            actors::{Actor, ActorRef, store_github_token_secrets},
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
    use hydra_common::{RepoName, api::v1 as api};

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
        let user = User::new(username.clone(), Some(42), false);
        handles
            .store
            .as_ref()
            .add_user(user, &ActorRef::test())
            .await?;
        store_github_token_secrets(&handles.state, &username, "token-123", "refresh-123").await;
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
                "https://github.com/octo/repo.git".to_string(),
                Some("main".to_string()),
                None,
            ),
        )
        .await?;
        let existing_patch = Patch::new(
            "Original".to_string(),
            "Original description".to_string(),
            "diff".to_string(),
            PatchStatus::Open,
            false,
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
            None,
            None,
            None,
        );

        let (patch_id, _) = handles
            .store
            .as_ref()
            .add_patch(existing_patch, &ActorRef::test())
            .await?;

        let request_patch = Patch::new(
            "Updated title".to_string(),
            "Updated description".to_string(),
            "diff".to_string(),
            PatchStatus::Open,
            false,
            Username::from("test-creator"),
            Vec::new(),
            repo_name,
            None,
            Some("feature".to_string()),
            None,
            None,
        );
        let request = api::patches::UpsertPatchRequest::new(request_patch.into());

        handles
            .state
            .upsert_patch_from_request(ActorRef::from(&actor), Some(patch_id.clone()), request)
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
                    r#"{"title":"New patch","head":"hydra-t-test","base":"main","body":"New patch description"}"#,
                );
            then.status(200)
                .header("content-type", "application/json")
                .json_body(github_pull_request_response(
                    99,
                    "hydra-t-test",
                    "main",
                    "https://example.com/pr/99",
                ));
        });

        let handles = test_state_with_github_api_base_url(github_server.base_url());
        let runner = start_test_automation_runner(&handles.state);
        let username = Username::from("octo");
        let user = User::new(username.clone(), Some(42), false);
        handles
            .store
            .as_ref()
            .add_user(user, &ActorRef::test())
            .await?;
        store_github_token_secrets(&handles.state, &username, "token-456", "refresh-456").await;
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
                "https://github.com/octo/repo.git".to_string(),
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
            .add_session(task.clone(), created_at, &ActorRef::test())
            .await?;
        task.status = Status::Running;
        handles
            .store
            .as_ref()
            .update_session(&task_id, task, &ActorRef::test())
            .await?;
        let patch = Patch::new(
            "New patch".to_string(),
            "New patch description".to_string(),
            "diff".to_string(),
            PatchStatus::Open,
            false,
            Username::from("test-creator"),
            Vec::new(),
            repo_name,
            None,
            Some("hydra-t-test".to_string()),
            None,
            None,
        );
        let request = api::patches::UpsertPatchRequest::new(patch.into());

        let (patch_id, _) = handles
            .state
            .upsert_patch_from_request(ActorRef::from(&actor), None, request)
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
        assert_eq!(github.head_ref.as_deref(), Some("hydra-t-test"));
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

        let patch1 = Patch::new(
            "First patch".to_string(),
            "desc".to_string(),
            "diff".to_string(),
            PatchStatus::Open,
            false,
            Username::from("test-creator"),
            Vec::new(),
            repo_name.clone(),
            None,
            Some("feature/foo".to_string()),
            None,
            None,
        );
        let request1 = api::patches::UpsertPatchRequest::new(patch1.into());
        let (patch1_id, _) = handles
            .state
            .upsert_patch_from_request(ActorRef::test(), None, request1)
            .await?;

        // Close the first patch
        let mut closed_patch = handles.store.get_patch(&patch1_id, false).await?.item;
        closed_patch.status = PatchStatus::Closed;
        handles
            .store
            .update_patch(&patch1_id, closed_patch, &ActorRef::test())
            .await?;

        // Creating a new patch with the same branch_name should succeed
        let patch2 = Patch::new(
            "Second patch".to_string(),
            "desc".to_string(),
            "diff".to_string(),
            PatchStatus::Open,
            false,
            Username::from("test-creator"),
            Vec::new(),
            repo_name,
            None,
            Some("feature/foo".to_string()),
            None,
            None,
        );
        let request2 = api::patches::UpsertPatchRequest::new(patch2.into());
        handles
            .state
            .upsert_patch_from_request(ActorRef::test(), None, request2)
            .await?;

        Ok(())
    }

    #[tokio::test]
    async fn upsert_patch_update_allows_same_branch() -> anyhow::Result<()> {
        let handles = test_state_handles();
        let repo_name = RepoName::new("octo", "repo")?;

        let patch1 = Patch::new(
            "First patch".to_string(),
            "desc".to_string(),
            "diff".to_string(),
            PatchStatus::Open,
            false,
            Username::from("test-creator"),
            Vec::new(),
            repo_name.clone(),
            None,
            Some("feature/foo".to_string()),
            None,
            None,
        );
        let request1 = api::patches::UpsertPatchRequest::new(patch1.into());
        let (patch1_id, _) = handles
            .state
            .upsert_patch_from_request(ActorRef::test(), None, request1)
            .await?;

        // Updating the same patch should succeed (the uniqueness check is only
        // on creates, not updates).
        let update_patch = Patch::new(
            "Updated title".to_string(),
            "desc".to_string(),
            "diff".to_string(),
            PatchStatus::Open,
            false,
            Username::from("test-creator"),
            Vec::new(),
            repo_name,
            None,
            Some("feature/foo".to_string()),
            None,
            None,
        );
        let request2 = api::patches::UpsertPatchRequest::new(update_patch.into());
        handles
            .state
            .upsert_patch_from_request(ActorRef::test(), Some(patch1_id), request2)
            .await?;

        Ok(())
    }

    #[tokio::test]
    async fn upsert_patch_preserves_creator_set_by_caller() -> anyhow::Result<()> {
        let handles = test_state_handles();
        let repo_name = RepoName::new("octo", "repo")?;

        let creator_username = Username::from("the-human");
        // Add a Running session so the actor-based running-job restriction
        // is satisfied for the upsert below.
        let mut task = sample_task();
        task.status = Status::Running;
        let (task_id, _) = handles
            .store
            .as_ref()
            .add_session(task, Utc::now(), &ActorRef::test())
            .await?;
        let (actor, _auth_token) = Actor::new_from_actor_id(
            crate::domain::actors::ActorId::Adhoc(task_id.clone()),
            creator_username.clone(),
            None,
        );
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
            creator_username.clone(),
            Vec::new(),
            repo_name,
            None,
            None,
            None,
            None,
        );
        let request = api::patches::UpsertPatchRequest::new(patch.into());

        let (patch_id, _) = handles
            .state
            .upsert_patch_from_request(ActorRef::from(&actor), None, request)
            .await?;

        let stored = handles.store.as_ref().get_patch(&patch_id, false).await?;
        assert_eq!(
            stored.item.creator, creator_username,
            "patch.creator should be preserved as set by the caller"
        );

        Ok(())
    }

    // -----------------------------------------------------------------
    // Review-author stamping & rejection tests
    // -----------------------------------------------------------------

    use crate::app::patches::UpsertPatchError;
    use crate::domain::actors::{ActorId as DomainActorId, ActorRef as DomainActorRef};
    use crate::domain::patches::Review;
    use hydra_common::ExternalSystem;
    use hydra_common::api::v1::patches::{UpsertPatch, UpsertReviewRequest};
    use hydra_common::principal::Principal;

    fn user_actor(name: &str) -> Actor {
        Actor::new_for_user(Username::from(name)).0
    }

    fn agent_actor(name: &str) -> Actor {
        Actor::new_from_actor_id(
            DomainActorId::Agent(hydra_common::api::v1::agents::AgentName::try_new(name).unwrap()),
            Username::from(name),
            None,
        )
        .0
    }

    fn adhoc_actor() -> Actor {
        let session_id = hydra_common::SessionId::new();
        Actor::new_from_actor_id(
            DomainActorId::Adhoc(session_id),
            Username::from("ad-hoc-creator"),
            None,
        )
        .0
    }

    async fn seed_patch_for_review_tests(
        handles: &crate::test_utils::TestStateHandles,
        creator: &str,
    ) -> anyhow::Result<hydra_common::PatchId> {
        let repo_name = hydra_common::RepoName::new("octo", "repo")?;
        let patch = crate::domain::patches::Patch::new(
            "for-review".to_string(),
            "desc".to_string(),
            "diff".to_string(),
            crate::domain::patches::PatchStatus::Open,
            false,
            Username::from(creator),
            Vec::new(),
            repo_name,
            None,
            None,
            None,
            None,
        );
        let (patch_id, _) = handles
            .store
            .as_ref()
            .add_patch(patch, &DomainActorRef::test())
            .await?;
        Ok(patch_id)
    }

    fn upsert_with_one_new_review(
        creator: &str,
        prior_patch: &crate::domain::patches::Patch,
        new_contents: &str,
    ) -> UpsertPatch {
        let mut upsert: UpsertPatch = api::patches::Patch::from(prior_patch.clone()).into();
        upsert.creator = hydra_common::api::v1::users::Username::from(creator);
        upsert.reviews.push(UpsertReviewRequest::new(
            new_contents.to_string(),
            true,
            Some(Utc::now()),
        ));
        upsert
    }

    #[tokio::test]
    async fn user_actor_stamps_review_author_as_principal_user() -> anyhow::Result<()> {
        let handles = crate::test_utils::test_state_handles();
        let creator = "alice";
        let patch_id = seed_patch_for_review_tests(&handles, creator).await?;

        let prior = handles.store.as_ref().get_patch(&patch_id, false).await?;
        let actor = user_actor(creator);
        let upsert = upsert_with_one_new_review(creator, &prior.item, "lgtm");

        let request = api::patches::UpsertPatchRequest::new(upsert);
        handles
            .state
            .upsert_patch_from_request(
                DomainActorRef::from(&actor),
                Some(patch_id.clone()),
                request,
            )
            .await?;

        let stored = handles.store.as_ref().get_patch(&patch_id, false).await?;
        assert_eq!(stored.item.reviews.len(), 1);
        assert_eq!(
            stored.item.reviews[0].author,
            Principal::User {
                name: hydra_common::api::v1::users::Username::try_new(creator).unwrap(),
            },
            "User actor should stamp Principal::User on the new review"
        );
        Ok(())
    }

    #[tokio::test]
    async fn agent_actor_stamps_review_author_as_principal_agent() -> anyhow::Result<()> {
        let handles = crate::test_utils::test_state_handles();
        let patch_id = seed_patch_for_review_tests(&handles, "alice").await?;

        let prior = handles.store.as_ref().get_patch(&patch_id, false).await?;
        let actor = agent_actor("reviewer");
        let upsert = upsert_with_one_new_review("alice", &prior.item, "approved by reviewer");

        let request = api::patches::UpsertPatchRequest::new(upsert);
        handles
            .state
            .upsert_patch_from_request(
                DomainActorRef::from(&actor),
                Some(patch_id.clone()),
                request,
            )
            .await?;

        let stored = handles.store.as_ref().get_patch(&patch_id, false).await?;
        assert_eq!(stored.item.reviews.len(), 1);
        assert_eq!(
            stored.item.reviews[0].author,
            Principal::Agent {
                name: hydra_common::api::v1::agents::AgentName::try_new("reviewer").unwrap(),
            },
            "Agent actor should stamp Principal::Agent on the new review"
        );
        Ok(())
    }

    #[tokio::test]
    async fn adhoc_actor_is_rejected_when_submitting_a_review() -> anyhow::Result<()> {
        let handles = crate::test_utils::test_state_handles();
        let patch_id = seed_patch_for_review_tests(&handles, "alice").await?;

        let prior = handles.store.as_ref().get_patch(&patch_id, false).await?;
        let actor = adhoc_actor();
        let upsert = upsert_with_one_new_review("alice", &prior.item, "lgtm");

        let request = api::patches::UpsertPatchRequest::new(upsert);
        let err = handles
            .state
            .upsert_patch_from_request(
                DomainActorRef::from(&actor),
                Some(patch_id.clone()),
                request,
            )
            .await
            .expect_err("ad-hoc actor must be rejected on review submission");

        match err {
            UpsertPatchError::InvalidActorForReview { reason, .. } => {
                assert_eq!(reason, "ad-hoc sessions cannot author reviews");
            }
            other => panic!("expected InvalidActorForReview, got {other:?}"),
        }
        Ok(())
    }

    #[tokio::test]
    async fn existing_review_author_is_preserved_across_no_op_update() -> anyhow::Result<()> {
        let handles = crate::test_utils::test_state_handles();
        let repo_name = hydra_common::RepoName::new("octo", "repo")?;

        // Seed a patch with one existing review authored by `bob`.
        let existing_submitted_at = Utc::now();
        let existing_review = Review::new(
            "earlier review".to_string(),
            false,
            Principal::User {
                name: hydra_common::api::v1::users::Username::try_new("bob").unwrap(),
            },
            Some(existing_submitted_at),
        );
        let patch = crate::domain::patches::Patch::new(
            "for-review".to_string(),
            "desc".to_string(),
            "diff".to_string(),
            crate::domain::patches::PatchStatus::Open,
            false,
            Username::from("alice"),
            vec![existing_review.clone()],
            repo_name,
            None,
            None,
            None,
            None,
        );
        let (patch_id, _) = handles
            .store
            .as_ref()
            .add_patch(patch, &DomainActorRef::test())
            .await?;

        // Now alice updates the patch with the existing review echoed back
        // through the request shape (which drops the author). The server's
        // matching logic must preserve bob's author rather than re-stamping
        // it as alice.
        let prior = handles.store.as_ref().get_patch(&patch_id, false).await?;
        let mut upsert: UpsertPatch = api::patches::Patch::from(prior.item.clone()).into();
        upsert.creator = hydra_common::api::v1::users::Username::from("alice");

        let actor = user_actor("alice");
        let request = api::patches::UpsertPatchRequest::new(upsert);
        handles
            .state
            .upsert_patch_from_request(
                DomainActorRef::from(&actor),
                Some(patch_id.clone()),
                request,
            )
            .await?;

        let stored = handles.store.as_ref().get_patch(&patch_id, false).await?;
        assert_eq!(stored.item.reviews.len(), 1);
        assert_eq!(
            stored.item.reviews[0].author,
            Principal::User {
                name: hydra_common::api::v1::users::Username::try_new("bob").unwrap(),
            },
            "existing review author must be preserved across no-op round-trip"
        );
        Ok(())
    }

    // Sanity smoke test: the request-shape `External` principal flows
    // unchanged through `From<Patch> for UpsertPatch` (which drops authors),
    // so we can't observe it on the request side here. This test instead
    // covers the format invariant: build a stored Review with an External
    // author and confirm it round-trips through serde.
    #[test]
    fn review_with_external_author_round_trips_through_serde() {
        let review = Review::new(
            "lgtm".to_string(),
            true,
            Principal::External {
                system: ExternalSystem::try_new("github").unwrap(),
                username: "octocat".to_string(),
            },
            Some(Utc::now()),
        );
        let json = serde_json::to_string(&review).unwrap();
        let back: Review = serde_json::from_str(&json).unwrap();
        assert_eq!(back, review);
    }

    /// The on-disk legacy shape (bare `author: "string"`) must still
    /// deserialize after a soft cutover. The row migration rewrites
    /// stored blobs, but until it has touched every row the runtime
    /// deserializer applies the same `parse_legacy_assignee` heuristic.
    /// Also doubles as a smoke test for the
    /// `review_author_legacy_decode` warn-log soak — the path must
    /// run without panicking and yield the expected typed Principal.
    #[test]
    fn review_deserialize_accepts_legacy_string_author_as_user() {
        let json = r#"{
            "contents": "old style review",
            "is_approved": true,
            "author": "alice",
            "submitted_at": null
        }"#;
        let review: Review = serde_json::from_str(json).unwrap();
        assert_eq!(
            review.author,
            Principal::User {
                name: hydra_common::api::v1::users::Username::try_new("alice").unwrap(),
            }
        );
    }

    #[test]
    fn review_deserialize_accepts_legacy_agents_path_as_agent() {
        let json = r#"{
            "contents": "old style agent review",
            "is_approved": true,
            "author": "agents/reviewer",
            "submitted_at": null
        }"#;
        let review: Review = serde_json::from_str(json).unwrap();
        assert_eq!(
            review.author,
            Principal::Agent {
                name: hydra_common::api::v1::agents::AgentName::try_new("reviewer").unwrap(),
            }
        );
    }
}
