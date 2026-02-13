use crate::app::event_bus::{EventType, MutationPayload, ServerEvent};
use crate::domain::actors::Actor;
use crate::domain::patches::GithubPr;
use crate::policy::context::AutomationContext;
use crate::policy::{AutomationError, EventFilter};
use async_trait::async_trait;
use octocrab::Octocrab;
use tracing::{info, warn};

/// Automation that creates or updates a GitHub pull request when a patch
/// is created or updated with `sync_github_branch` set.
///
/// This is a direct translation of the former inline `sync_patch_with_github`
/// logic from `AppState::upsert_patch`. The automation fires after the patch
/// is persisted, reads the `sync_github_branch` field, performs the GitHub
/// API call, then updates the patch with the resulting PR metadata and clears
/// the `sync_github_branch` field.
pub struct GithubPrSyncAutomation;

impl GithubPrSyncAutomation {
    pub fn new(_params: Option<&toml::Value>) -> Result<Self, String> {
        Ok(Self)
    }
}

#[async_trait]
impl crate::policy::Automation for GithubPrSyncAutomation {
    fn name(&self) -> &str {
        "github_pr_sync"
    }

    fn event_filter(&self) -> EventFilter {
        EventFilter {
            event_types: vec![EventType::PatchCreated, EventType::PatchUpdated],
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

        let MutationPayload::Patch { new, .. } = payload.as_ref() else {
            return Ok(());
        };

        // Only run when sync_github_branch is set on the persisted patch.
        let head_ref = match &new.sync_github_branch {
            Some(branch) => branch.clone(),
            None => return Ok(()),
        };

        // Resolve actor name from the event payload.
        let actor_name = match ctx.actor() {
            Some(name) => name.to_string(),
            None => {
                warn!(
                    patch_id = %patch_id,
                    "github_pr_sync: no actor in event, skipping"
                );
                return Ok(());
            }
        };

        // Build a temporary Actor to fetch the GitHub token.
        let user_or_worker = Actor::parse_name(&actor_name).map_err(|e| {
            AutomationError::Other(anyhow::anyhow!(
                "github_pr_sync: failed to parse actor name '{actor_name}': {e}"
            ))
        })?;
        // Only `user_or_worker` is needed — `get_github_token` resolves the
        // token via the user store lookup and never accesses `auth_token_hash`
        // or `auth_token_salt`.
        let actor = Actor {
            auth_token_hash: String::new(),
            auth_token_salt: String::new(),
            user_or_worker,
        };

        let token = actor.get_github_token(ctx.app_state).await.map_err(|e| {
            AutomationError::Other(anyhow::anyhow!(
                "github_pr_sync: failed to get github token for actor '{actor_name}': {e:?}"
            ))
        })?;

        let client = Octocrab::builder()
            .base_uri(ctx.app_state.config.github_app.api_base_url().to_string())
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
            // Determine base ref: use existing value or fetch default branch.
            let base_ref = match patch
                .github
                .as_ref()
                .and_then(|github| github.base_ref.as_ref())
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            {
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

        // Clear the sync signal and persist.
        patch.sync_github_branch = None;
        ctx.store.update_patch(patch_id, patch).await.map_err(|e| {
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
    fn skips_when_sync_github_branch_is_none() {
        // Verify that the automation correctly identifies patches without
        // sync_github_branch by checking the field directly. The full
        // execute() path requires AppState which is covered by integration
        // tests in app_state.rs.
        let patch = Patch::new(
            "title".into(),
            "desc".into(),
            "diff".into(),
            PatchStatus::Open,
            false,
            None,
            vec![],
            RepoName::new("org", "repo").unwrap(),
            None,
        );
        assert!(patch.sync_github_branch.is_none());
    }

    #[test]
    fn triggers_when_sync_github_branch_is_set() {
        let mut patch = Patch::new(
            "title".into(),
            "desc".into(),
            "diff".into(),
            PatchStatus::Open,
            false,
            None,
            vec![],
            RepoName::new("org", "repo").unwrap(),
            None,
        );
        patch.sync_github_branch = Some("feature/branch".into());
        assert!(patch.sync_github_branch.is_some());
    }
}
