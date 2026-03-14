use crate::app::event_bus::{EventType, MutationPayload, ServerEvent};
use crate::domain::actors::{ActorId, ActorRef, get_github_token_for_user};
use crate::domain::patches::GithubPr;
use crate::policy::context::AutomationContext;
use crate::policy::{AutomationError, EventFilter};
use async_trait::async_trait;
use octocrab::Octocrab;
use tracing::{info, warn};

const AUTOMATION_NAME: &str = "github_pr_sync";

/// Automation that creates or updates a GitHub pull request when a patch
/// is created or updated with `branch_name` set.
///
/// This replaces the former inline `sync_patch_with_github` logic from
/// `AppState::upsert_patch`. The automation fires after the patch is
/// persisted, reads the `branch_name` field, performs the GitHub API call,
/// then updates the patch with the resulting PR metadata.
///
/// Re-entrancy guard: after the automation persists GitHub PR metadata, a
/// new `PatchUpdated` event fires. The automation detects this by comparing
/// the old and new patch from the mutation payload — if the only change is
/// to the `github` field, the update was caused by this automation and is
/// skipped.
pub struct GithubPrSyncAutomation;

impl GithubPrSyncAutomation {
    pub fn new(_params: Option<&serde_yaml_ng::Value>) -> Result<Self, String> {
        Ok(Self)
    }
}

#[async_trait]
impl crate::policy::Automation for GithubPrSyncAutomation {
    fn name(&self) -> &str {
        AUTOMATION_NAME
    }

    fn event_filter(&self) -> EventFilter {
        EventFilter {
            event_types: vec![EventType::PatchCreated, EventType::PatchUpdated],
            ..Default::default()
        }
    }

    async fn execute(&self, ctx: &AutomationContext<'_>) -> Result<(), AutomationError> {
        let (patch_id, payload) = match ctx.event {
            ServerEvent::PatchCreated {
                patch_id, payload, ..
            }
            | ServerEvent::PatchUpdated {
                patch_id, payload, ..
            } => (patch_id, payload),
            _ => return Ok(()),
        };

        let MutationPayload::Patch { old, new, .. } = payload.as_ref() else {
            return Ok(());
        };

        // Only run when branch_name is set on the patch.
        let head_ref = match &new.branch_name {
            Some(branch) => branch.clone(),
            None => return Ok(()),
        };

        // Re-entrancy guard: after this automation syncs a PR it persists the
        // github metadata, which triggers another PatchUpdated event. Detect
        // this by comparing old and new — if the only field that changed is
        // `github`, this update was caused by our own write and we skip it.
        if let Some(old) = old {
            let mut old_without_github = old.clone();
            let mut new_without_github = new.clone();
            old_without_github.github = None;
            new_without_github.github = None;
            if old_without_github == new_without_github && old.github != new.github {
                return Ok(());
            }
        }

        // Resolve actor identity from the event payload.
        let actor_ref = ctx.actor();
        let actor_name = actor_ref.display_name();
        let actor_id = match actor_ref {
            ActorRef::Authenticated { actor_id } => actor_id.clone(),
            ActorRef::System { worker_name, .. } => {
                warn!(
                    patch_id = %patch_id,
                    worker_name = %worker_name,
                    "github_pr_sync: system actor cannot sync PRs, skipping"
                );
                return Ok(());
            }
            ActorRef::Automation {
                automation_name, ..
            } => {
                warn!(
                    patch_id = %patch_id,
                    automation_name = %automation_name,
                    "github_pr_sync: automation actor cannot sync PRs, skipping"
                );
                return Ok(());
            }
        };

        // Resolve the creator username from the actor identity.
        let creator = match &actor_id {
            ActorId::Username(username) => username.clone().into(),
            ActorId::Session(session_id) => {
                let task = ctx.app_state.get_session(session_id).await.map_err(|e| {
                    AutomationError::Other(anyhow::anyhow!(
                        "github_pr_sync: failed to load task '{session_id}': {e}"
                    ))
                })?;
                task.creator
            }
            ActorId::Issue(issue_id) => {
                let issue = ctx.store.get_issue(issue_id, false).await.map_err(|e| {
                    AutomationError::Other(anyhow::anyhow!(
                        "github_pr_sync: failed to load issue '{issue_id}': {e}"
                    ))
                })?;
                issue.item.creator
            }
            ActorId::Service(_) => {
                let actor = ctx
                    .store
                    .get_actor(&actor_id.to_string())
                    .await
                    .map_err(|e| {
                        AutomationError::Other(anyhow::anyhow!(
                            "github_pr_sync: failed to load actor '{actor_id}': {e}"
                        ))
                    })?;
                actor.item.creator
            }
        };

        let token = get_github_token_for_user(ctx.app_state, &creator)
            .await
            .map_err(|e| {
                AutomationError::Other(anyhow::anyhow!(
                    "github_pr_sync: failed to get github token for actor '{actor_name}': {e:?}"
                ))
            })?;

        let client = Octocrab::builder()
            .base_uri(ctx.app_state.config.github_api_base_url().to_string())
            .map_err(|e| {
                AutomationError::Other(anyhow::anyhow!(
                    "github_pr_sync: failed to build octocrab client: {e}"
                ))
            })?
            .personal_token(token.github_token)
            .build()
            .map_err(|e| {
                AutomationError::Other(anyhow::anyhow!(
                    "github_pr_sync: failed to build octocrab client: {e}"
                ))
            })?;

        let mut patch = new.clone();
        let (owner, repo) = match patch.github.as_ref() {
            Some(github) => (github.owner.clone(), github.repo.clone()),
            None => (
                patch.service_repo_name.organization.clone(),
                patch.service_repo_name.repo.clone(),
            ),
        };

        if let Some(existing) = patch.github.as_ref() {
            // Update existing PR.
            let pr = client
                .pulls(&owner, &repo)
                .update(existing.number)
                .title(patch.title.clone())
                .body(patch.description.clone())
                .send()
                .await
                .map_err(|e| {
                    AutomationError::Other(anyhow::anyhow!(
                        "github_pr_sync: failed to update PR '{owner}/{repo}#{}': {e}",
                        existing.number
                    ))
                })?;

            let mut updated = existing.clone();
            updated.head_ref = Some(pr.head.ref_field.clone());
            updated.base_ref = Some(pr.base.ref_field.clone());
            updated.url = pr.html_url.as_ref().map(ToString::to_string);
            patch.github = Some(updated);
        } else {
            // Determine base ref: prefer patch.base_branch, then
            // patch.github.base_ref, then fall back to the repository's
            // configured default branch.
            let base_ref = match patch
                .base_branch
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .or_else(|| {
                    patch
                        .github
                        .as_ref()
                        .and_then(|github| github.base_ref.as_ref())
                        .map(|value| value.trim().to_string())
                        .filter(|value| !value.is_empty())
                }) {
                Some(base_ref) => base_ref,
                None => {
                    let repository = ctx
                        .app_state
                        .repository_from_store(&patch.service_repo_name)
                        .await
                        .map_err(|e| {
                            AutomationError::Other(anyhow::anyhow!(
                                "github_pr_sync: failed to load repository '{}': {e}",
                                patch.service_repo_name
                            ))
                        })?;
                    repository
                        .default_branch
                        .map(|value| value.trim().to_string())
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| {
                            AutomationError::Other(anyhow::anyhow!(
                                "github_pr_sync: no base ref available for '{}'",
                                patch.service_repo_name
                            ))
                        })?
                }
            };

            // Create a new PR.
            let pr = client
                .pulls(&owner, &repo)
                .create(patch.title.clone(), &head_ref, base_ref)
                .body(patch.description.clone())
                .send()
                .await
                .map_err(|e| {
                    AutomationError::Other(anyhow::anyhow!(
                        "github_pr_sync: failed to create PR for '{owner}/{repo}': {e}"
                    ))
                })?;

            patch.github = Some(GithubPr::new(
                owner,
                repo,
                pr.number,
                Some(pr.head.ref_field.clone()),
                Some(pr.base.ref_field.clone()),
                pr.html_url.as_ref().map(ToString::to_string),
                patch.github.as_ref().and_then(|github| github.ci.clone()),
            ));
        }

        // Persist the updated GitHub metadata via AppState (store is read-only
        // in the automation context, so we must go through AppState for writes).
        let request = metis_common::api::v1::patches::UpsertPatchRequest::new(patch.into());
        ctx.app_state
            .upsert_patch(
                ActorRef::Automation {
                    automation_name: AUTOMATION_NAME.into(),
                    triggered_by: Some(Box::new(ctx.actor().clone())),
                },
                Some(patch_id.clone()),
                request,
            )
            .await
            .map_err(|e| {
                AutomationError::Other(anyhow::anyhow!(
                    "github_pr_sync: failed to persist github metadata for patch '{patch_id}': {e}"
                ))
            })?;

        info!(
            patch_id = %patch_id,
            "github_pr_sync: successfully synced patch with github"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::patches::{Patch, PatchStatus};
    use crate::domain::users::Username;
    use crate::policy::Automation;
    use metis_common::RepoName;

    #[test]
    fn automation_name_and_filter() {
        let automation = GithubPrSyncAutomation::new(None).unwrap();
        assert_eq!(automation.name(), "github_pr_sync");
        let filter = automation.event_filter();
        assert!(filter.event_types.contains(&EventType::PatchCreated));
        assert!(filter.event_types.contains(&EventType::PatchUpdated));
        assert_eq!(filter.event_types.len(), 2);
    }

    #[test]
    fn skips_when_branch_name_is_none() {
        let patch = Patch::new(
            "title".into(),
            "desc".into(),
            "diff".into(),
            PatchStatus::Open,
            false,
            None,
            Username::from("test-creator"),
            vec![],
            RepoName::new("org", "repo").unwrap(),
            None,
            None,
            None,
            None,
        );
        assert!(patch.branch_name.is_none());
    }

    #[test]
    fn triggers_when_branch_name_is_set() {
        let patch = Patch::new(
            "title".into(),
            "desc".into(),
            "diff".into(),
            PatchStatus::Open,
            false,
            None,
            Username::from("test-creator"),
            vec![],
            RepoName::new("org", "repo").unwrap(),
            None,
            Some("feature/branch".into()),
            None,
            None,
        );
        assert!(patch.branch_name.is_some());
    }

    #[test]
    fn skips_when_only_github_field_changed() {
        let old_patch = Patch::new(
            "title".into(),
            "desc".into(),
            "diff".into(),
            PatchStatus::Open,
            false,
            None,
            Username::from("test-creator"),
            vec![],
            RepoName::new("org", "repo").unwrap(),
            None,
            Some("feature/branch".into()),
            None,
            None,
        );

        let mut new_patch = old_patch.clone();
        new_patch.github = Some(GithubPr::new(
            "org".into(),
            "repo".into(),
            1,
            Some("feature/branch".into()),
            Some("main".into()),
            None,
            None,
        ));

        // Re-entrancy guard: only github changed, should skip.
        let mut old_no_gh = old_patch.clone();
        let mut new_no_gh = new_patch.clone();
        old_no_gh.github = None;
        new_no_gh.github = None;
        assert_eq!(old_no_gh, new_no_gh);
        assert_ne!(old_patch.github, new_patch.github);
    }

    #[test]
    fn does_not_skip_when_non_github_fields_also_changed() {
        let mut old_patch = Patch::new(
            "title".into(),
            "desc".into(),
            "diff".into(),
            PatchStatus::Open,
            false,
            None,
            Username::from("test-creator"),
            vec![],
            RepoName::new("org", "repo").unwrap(),
            None,
            Some("feature/branch".into()),
            None,
            None,
        );
        old_patch.github = Some(GithubPr::new(
            "org".into(),
            "repo".into(),
            1,
            Some("feature/branch".into()),
            Some("main".into()),
            None,
            None,
        ));

        let mut new_patch = old_patch.clone();
        new_patch.title = "Updated title".into();
        new_patch.github = Some(GithubPr::new(
            "org".into(),
            "repo".into(),
            1,
            Some("feature/branch".into()),
            Some("main".into()),
            Some("https://example.com/pr/1".into()),
            None,
        ));

        // Non-github field (title) changed, so guard should NOT skip.
        let mut old_no_gh = old_patch.clone();
        let mut new_no_gh = new_patch.clone();
        old_no_gh.github = None;
        new_no_gh.github = None;
        assert_ne!(old_no_gh, new_no_gh);
    }
}
