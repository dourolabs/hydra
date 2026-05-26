use crate::{
    domain::actors::ActorRef,
    store::{ReadOnlyStore, StoreError},
};
use hydra_common::{
    RepoName,
    api::v1::repositories::{AssigneeRef, MergePolicy, SearchRepositoriesQuery},
};

use super::app_state::AppState;
use super::{Repository, RepositoryError, RepositoryRecord};

impl AppState {
    pub async fn list_repositories(
        &self,
        query: &SearchRepositoriesQuery,
    ) -> Result<Vec<RepositoryRecord>, RepositoryError> {
        let store = self.store.as_ref();
        let repositories = store
            .list_repositories(query)
            .await
            .map_err(|source| RepositoryError::Store { source })?;

        Ok(repositories
            .into_iter()
            .map(|(name, repository)| RepositoryRecord::from((name, repository.item)))
            .collect())
    }

    pub async fn delete_repository(
        &self,
        name: &RepoName,
        actor: ActorRef,
    ) -> Result<RepositoryRecord, RepositoryError> {
        // Get the repository before deleting to return it
        // Use include_deleted: true since we need to access the repository to mark it as deleted
        let current =
            self.store
                .get_repository(name, true)
                .await
                .map_err(|source| match source {
                    StoreError::RepositoryNotFound(_) => RepositoryError::NotFound(name.clone()),
                    other => RepositoryError::Store { source: other },
                })?;

        self.store
            .delete_repository(name, actor)
            .await
            .map_err(|source| match source {
                StoreError::RepositoryNotFound(_) => RepositoryError::NotFound(name.clone()),
                other => RepositoryError::Store { source: other },
            })?;

        self.service_state.clear_cache(name).await;

        let mut deleted_repo = current.item;
        deleted_repo.deleted = true;
        Ok(RepositoryRecord::from((name.clone(), deleted_repo)))
    }

    pub async fn create_repository(
        &self,
        name: RepoName,
        config: Repository,
        actor: ActorRef,
    ) -> Result<RepositoryRecord, RepositoryError> {
        // Phase 5a: validate every static `Principal` in the incoming
        // merge_policy via `Store::principal_exists` BEFORE persisting
        // the row, per design §4.2 / §4.5. Validation runs only on the
        // new/incoming value — old configs are not retroactively checked,
        // which matches `upsert_issue`'s assignee-validation behaviour.
        self.validate_merge_policy_principals(config.merge_policy.as_ref())
            .await?;

        self.store
            .add_repository(name.clone(), config.clone(), actor)
            .await
            .map_err(|source| match source {
                StoreError::RepositoryAlreadyExists(name) => RepositoryError::AlreadyExists(name),
                other => RepositoryError::Store { source: other },
            })?;

        Ok(RepositoryRecord::from((name, config)))
    }

    pub async fn update_repository(
        &self,
        name: RepoName,
        config: Repository,
        actor: ActorRef,
    ) -> Result<RepositoryRecord, RepositoryError> {
        self.validate_merge_policy_principals(config.merge_policy.as_ref())
            .await?;

        self.store
            .update_repository(name.clone(), config.clone(), actor)
            .await
            .map_err(|source| match source {
                StoreError::RepositoryNotFound(_) => RepositoryError::NotFound(name.clone()),
                StoreError::RepositoryAlreadyExists(_) => {
                    RepositoryError::AlreadyExists(name.clone())
                }
                other => RepositoryError::Store { source: other },
            })?;

        self.service_state.clear_cache(&name).await;

        Ok(RepositoryRecord::from((name, config)))
    }

    pub async fn repository_from_store(&self, name: &RepoName) -> Result<Repository, StoreError> {
        let store = self.store.as_ref();
        // Use include_deleted: false since API callers should not see deleted repositories
        store
            .get_repository(name, false)
            .await
            .map(|repo| repo.item)
    }

    /// Walk every static `Principal` in the policy's reviewer groups and
    /// `mergers` rule and ensure it resolves to a real Hydra row via
    /// `Store::principal_exists`. Dynamic refs (e.g. `@patch.author`) are
    /// skipped: they're resolved at merge-attempt time against the patch,
    /// not the user/agent tables. `External` principals are accepted
    /// without a DB lookup (format-only validation, per design §4.5).
    async fn validate_merge_policy_principals(
        &self,
        policy: Option<&MergePolicy>,
    ) -> Result<(), RepositoryError> {
        let Some(policy) = policy else {
            return Ok(());
        };
        for group in &policy.reviewers {
            for principal in &group.any_of {
                self.validate_assignee_ref(principal).await?;
            }
        }
        if let Some(mergers) = &policy.mergers {
            for principal in &mergers.any_of {
                self.validate_assignee_ref(principal).await?;
            }
        }
        Ok(())
    }

    async fn validate_assignee_ref(&self, principal: &AssigneeRef) -> Result<(), RepositoryError> {
        let AssigneeRef::Static(p) = principal else {
            return Ok(());
        };
        let exists = self
            .store
            .as_ref()
            .principal_exists(p)
            .await
            .map_err(|source| RepositoryError::PrincipalLookup {
                principal: p.clone(),
                source,
            })?;
        if !exists {
            return Err(RepositoryError::UnknownPrincipal {
                principal: p.clone(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_state;
    use hydra_common::Principal;
    use hydra_common::api::v1::agents::AgentName;
    use hydra_common::api::v1::repositories::{DynamicRef, MergerRule, ReviewerGroup};
    use hydra_common::api::v1::users::Username as ApiUsername;
    use hydra_common::principal::ExternalSystem;

    fn repo_name() -> RepoName {
        RepoName::new("dourolabs", "hydra").unwrap()
    }

    fn repo_with_policy(policy: Option<MergePolicy>) -> Repository {
        let mut repo = Repository::new("https://example.com/repo.git".to_string(), None, None);
        repo.merge_policy = policy;
        repo
    }

    fn user_ref(name: &str) -> AssigneeRef {
        AssigneeRef::Static(Principal::User {
            name: ApiUsername::try_new(name).unwrap(),
        })
    }

    fn agent_ref(name: &str) -> AssigneeRef {
        AssigneeRef::Static(Principal::Agent {
            name: AgentName::try_new(name).unwrap(),
        })
    }

    async fn seed_user(state: &AppState, username: &str) {
        state
            .store
            .add_user(
                crate::domain::users::User::new(
                    crate::domain::users::Username::from(username),
                    None,
                    false,
                ),
                ActorRef::test(),
            )
            .await
            .unwrap();
    }

    async fn seed_agent(state: &AppState, name: &str) {
        state
            .store
            .add_agent(crate::domain::agents::Agent::new(
                name.to_string(),
                format!("/agents/{name}/prompt.md"),
                None,
                3,
                4,
                false,
                false,
                Vec::new(),
            ))
            .await
            .unwrap();
    }

    /// Phase 5a §4.2 footgun: a config that writes `Principal::User { name: "swe" }`
    /// when `swe` is in fact an agent must fail with 400. The validation runs on
    /// the User table, not the Agent table, so the agent's existence does not
    /// rescue the wrong-kind reference.
    #[tokio::test]
    async fn create_repository_rejects_user_principal_when_only_agent_with_that_name_exists() {
        let state = test_state();
        seed_agent(&state, "swe").await;
        let config = repo_with_policy(Some(MergePolicy {
            reviewers: vec![ReviewerGroup {
                label: None,
                any_of: vec![user_ref("swe")],
                count: 1,
                exclude_author: true,
            }],
            mergers: None,
        }));

        let err = state
            .create_repository(repo_name(), config, ActorRef::test())
            .await
            .unwrap_err();
        match err {
            RepositoryError::UnknownPrincipal { principal } => {
                assert_eq!(principal.to_string(), "users/swe");
            }
            other => panic!("expected UnknownPrincipal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_repository_rejects_unknown_agent_in_mergers() {
        let state = test_state();
        // No `ghost` agent or user in the store.
        let config = repo_with_policy(Some(MergePolicy {
            reviewers: vec![],
            mergers: Some(MergerRule {
                any_of: vec![agent_ref("ghost")],
            }),
        }));

        let err = state
            .create_repository(repo_name(), config, ActorRef::test())
            .await
            .unwrap_err();
        match err {
            RepositoryError::UnknownPrincipal { principal } => {
                assert_eq!(principal.to_string(), "agents/ghost");
            }
            other => panic!("expected UnknownPrincipal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_repository_accepts_known_user_and_known_agent() {
        let state = test_state();
        seed_user(&state, "alice").await;
        seed_agent(&state, "swe").await;
        let config = repo_with_policy(Some(MergePolicy {
            reviewers: vec![ReviewerGroup {
                label: None,
                any_of: vec![user_ref("alice"), agent_ref("swe")],
                count: 1,
                exclude_author: true,
            }],
            mergers: None,
        }));

        let record = state
            .create_repository(repo_name(), config.clone(), ActorRef::test())
            .await
            .expect("create_repository should accept known principals");
        assert_eq!(record.name, repo_name());
        assert_eq!(record.repository.merge_policy, config.merge_policy);
    }

    /// Per design §4.5, `External` principals are not validated against
    /// any DB table — they live in an external identity provider by
    /// definition. The config write must succeed even though no
    /// `external/github/anyone` row exists locally.
    #[tokio::test]
    async fn create_repository_accepts_external_principal_without_db_lookup() {
        let state = test_state();
        let github = ExternalSystem::try_new("github").unwrap();
        let config = repo_with_policy(Some(MergePolicy {
            reviewers: vec![ReviewerGroup {
                label: None,
                any_of: vec![AssigneeRef::Static(Principal::External {
                    system: github,
                    username: "anyone".to_string(),
                })],
                count: 1,
                exclude_author: true,
            }],
            mergers: None,
        }));

        state
            .create_repository(repo_name(), config, ActorRef::test())
            .await
            .expect("external principals must succeed without DB lookup");
    }

    /// Dynamic refs (`@patch.author`) are resolved at merge-attempt time
    /// against the patch, not the user/agent tables — config writes
    /// referencing them must never trigger `principal_exists`.
    #[tokio::test]
    async fn create_repository_accepts_dynamic_ref_without_principal_lookup() {
        let state = test_state();
        let config = repo_with_policy(Some(MergePolicy {
            reviewers: vec![],
            mergers: Some(MergerRule {
                any_of: vec![AssigneeRef::Dynamic(DynamicRef::PatchAuthor)],
            }),
        }));

        state
            .create_repository(repo_name(), config, ActorRef::test())
            .await
            .expect("dynamic refs must not be validated against the user/agent tables");
    }

    /// Updates are validated the same way creates are.
    #[tokio::test]
    async fn update_repository_rejects_unknown_user() {
        let state = test_state();
        seed_user(&state, "alice").await;
        // First create the repo with a known principal so the update path has
        // a row to mutate.
        state
            .create_repository(
                repo_name(),
                repo_with_policy(Some(MergePolicy {
                    reviewers: vec![ReviewerGroup {
                        label: None,
                        any_of: vec![user_ref("alice")],
                        count: 1,
                        exclude_author: true,
                    }],
                    mergers: None,
                })),
                ActorRef::test(),
            )
            .await
            .unwrap();

        // Now try to update the policy to reference a non-existent user.
        let err = state
            .update_repository(
                repo_name(),
                repo_with_policy(Some(MergePolicy {
                    reviewers: vec![ReviewerGroup {
                        label: None,
                        any_of: vec![user_ref("ghost")],
                        count: 1,
                        exclude_author: true,
                    }],
                    mergers: None,
                })),
                ActorRef::test(),
            )
            .await
            .unwrap_err();
        match err {
            RepositoryError::UnknownPrincipal { principal } => {
                assert_eq!(principal.to_string(), "users/ghost");
            }
            other => panic!("expected UnknownPrincipal, got {other:?}"),
        }
    }

    /// Repos without a merge_policy don't trigger validation at all.
    #[tokio::test]
    async fn create_repository_with_no_policy_skips_validation() {
        let state = test_state();
        state
            .create_repository(repo_name(), repo_with_policy(None), ActorRef::test())
            .await
            .expect("no policy = no validation");
    }
}
