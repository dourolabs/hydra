use async_trait::async_trait;

use crate::app::event_bus::{MutationPayload, ServerEvent};
use crate::domain::patches::{GithubPr, Patch, PatchStatus};
use crate::policy::context::AutomationContext;
use crate::policy::{Automation, AutomationError, EventFilter};

/// Synchronizes patches with GitHub pull requests when patches are created or
/// updated. When a patch has GitHub metadata, this automation updates the
/// corresponding PR's title and description to match.
///
/// This automation uses the GitHub App installation client (not user tokens) so
/// it can run asynchronously after the patch is persisted.
pub struct GithubPrSyncAutomation;

impl GithubPrSyncAutomation {
    pub fn new(_params: Option<&toml::Value>) -> Result<Self, String> {
        Ok(Self)
    }
}

#[async_trait]
impl Automation for GithubPrSyncAutomation {
    fn name(&self) -> &str {
        "github_pr_sync"
    }

    fn event_filter(&self) -> EventFilter {
        EventFilter {
            event_types: vec![
                super::patch_created_discriminant(),
                super::patch_updated_discriminant(),
            ],
        }
    }

    async fn execute(&self, ctx: &AutomationContext<'_>) -> Result<(), AutomationError> {
        let (patch_id, payload) = match ctx.event {
            ServerEvent::PatchCreated {
                patch_id, payload, ..
            } => (patch_id, payload),
            ServerEvent::PatchUpdated {
                patch_id, payload, ..
            } => (patch_id, payload),
            _ => return Ok(()),
        };

        let new_patch = match payload.as_ref() {
            MutationPayload::Patch { new, .. } => new,
            _ => return Ok(()),
        };

        // Only sync patches that already have GitHub metadata (PR was created
        // during the upsert_patch call via sync_patch_with_github).
        let Some(github) = new_patch.github.as_ref() else {
            return Ok(());
        };

        // Skip closed/merged patches — no need to sync.
        if matches!(new_patch.status, PatchStatus::Closed | PatchStatus::Merged) {
            return Ok(());
        }

        let Some(client) = select_github_installation_client(ctx, github).await? else {
            tracing::debug!(
                patch_id = %patch_id,
                "no GitHub App installation available; skipping PR sync automation"
            );
            return Ok(());
        };

        // Update the PR title and description to match the patch.
        match client
            .pulls(&github.owner, &github.repo)
            .update(github.number)
            .title(&new_patch.title)
            .body(&new_patch.description)
            .send()
            .await
        {
            Ok(_) => {
                tracing::info!(
                    patch_id = %patch_id,
                    pr_number = github.number,
                    "updated GitHub PR from patch"
                );
            }
            Err(err) => {
                tracing::warn!(
                    patch_id = %patch_id,
                    pr_number = github.number,
                    error = %err,
                    "failed to update GitHub PR from automation; will retry on next event"
                );
            }
        }

        Ok(())
    }
}

async fn select_github_installation_client(
    ctx: &AutomationContext<'_>,
    github: &GithubPr,
) -> Result<Option<octocrab::Octocrab>, AutomationError> {
    let Some(app_client) = ctx.app_state.github_app.as_ref() else {
        return Ok(None);
    };

    let installation = match app_client
        .apps()
        .get_repository_installation(&github.owner, &github.repo)
        .await
    {
        Ok(installation) => installation,
        Err(err) => {
            tracing::warn!(
                owner = %github.owner,
                repo = %github.repo,
                error = %err,
                "failed to lookup GitHub App installation for PR sync"
            );
            return Ok(None);
        }
    };

    let (installation_client, _token) =
        match app_client.installation_and_token(installation.id).await {
            Ok(result) => result,
            Err(err) => {
                tracing::warn!(
                    owner = %github.owner,
                    repo = %github.repo,
                    installation_id = %installation.id,
                    error = %err,
                    "failed to fetch GitHub App installation token for PR sync"
                );
                return Ok(None);
            }
        };

    Ok(Some(installation_client))
}

/// Synchronize a patch with GitHub by creating or updating a pull request.
///
/// This is the extracted logic from `AppState::sync_patch_with_github`, kept as
/// a standalone function so it can be called inline from `upsert_patch` when
/// `sync_github_branch` is provided. It uses the caller-provided `Octocrab`
/// client (built from the user's personal token).
pub async fn sync_patch_with_github(
    app_state: &crate::app::AppState,
    client: &octocrab::Octocrab,
    patch: &mut Patch,
    head_ref: &str,
) -> Result<(), SyncError> {
    let (owner, repo) = match patch.github.as_ref() {
        Some(github) => (github.owner.clone(), github.repo.clone()),
        None => (
            patch.service_repo_name.organization.clone(),
            patch.service_repo_name.repo.clone(),
        ),
    };

    if let Some(existing) = patch.github.as_ref() {
        let pr = client
            .pulls(&owner, &repo)
            .update(existing.number)
            .title(patch.title.clone())
            .body(patch.description.clone())
            .send()
            .await
            .map_err(|source| SyncError::PullRequestUpdate {
                source,
                owner: owner.clone(),
                repo: repo.clone(),
                number: existing.number,
            })?;

        let mut updated = existing.clone();
        updated.head_ref = Some(pr.head.ref_field.clone());
        updated.base_ref = Some(pr.base.ref_field.clone());
        updated.url = pr.html_url.as_ref().map(ToString::to_string);
        patch.github = Some(updated);
        return Ok(());
    }

    let base_ref = match patch
        .github
        .as_ref()
        .and_then(|github| github.base_ref.as_ref())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        Some(base_ref) => base_ref,
        None => {
            let repository = app_state
                .repository_from_store(&patch.service_repo_name)
                .await
                .map_err(|source| SyncError::RepositoryLookup {
                    source,
                    repo_name: patch.service_repo_name.clone(),
                })?;
            repository
                .default_branch
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .ok_or(SyncError::BaseRefMissing)?
        }
    };

    let pr = client
        .pulls(&owner, &repo)
        .create(patch.title.clone(), head_ref, base_ref)
        .body(patch.description.clone())
        .send()
        .await
        .map_err(|source| SyncError::PullRequestCreate {
            source,
            owner: owner.clone(),
            repo: repo.clone(),
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

    Ok(())
}

/// Build an Octocrab client authenticated with a user's personal GitHub token.
pub async fn github_user_client(
    app_state: &crate::app::AppState,
    actor: &crate::domain::actors::Actor,
) -> Result<octocrab::Octocrab, SyncError> {
    let token = actor
        .get_github_token(app_state)
        .await
        .map_err(|err| SyncError::TokenLookup {
            actor: actor.name(),
            message: err.message().to_string(),
        })?;

    octocrab::Octocrab::builder()
        .base_uri(app_state.config.github_app.api_base_url().to_string())
        .map_err(|source| SyncError::ClientBuild {
            source,
            actor: actor.name(),
        })?
        .personal_token(token.github_token)
        .build()
        .map_err(|source| SyncError::ClientBuild {
            source,
            actor: actor.name(),
        })
}

/// Errors from GitHub PR synchronization.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("failed to load github token for actor '{actor}': {message}")]
    TokenLookup { actor: String, message: String },

    #[error("failed to create github client for actor '{actor}'")]
    ClientBuild {
        #[source]
        source: octocrab::Error,
        actor: String,
    },

    #[error("github sync requires a base ref")]
    BaseRefMissing,

    #[error("failed to load repository '{repo_name}' for github sync")]
    RepositoryLookup {
        #[source]
        source: crate::store::StoreError,
        repo_name: metis_common::RepoName,
    },

    #[error("failed to update github pull request '{owner}/{repo}#{number}'")]
    PullRequestUpdate {
        #[source]
        source: octocrab::Error,
        owner: String,
        repo: String,
        number: u64,
    },

    #[error("failed to create github pull request for '{owner}/{repo}'")]
    PullRequestCreate {
        #[source]
        source: octocrab::Error,
        owner: String,
        repo: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::event_bus::MutationPayload;
    use crate::domain::patches::{GithubPr, Patch, PatchStatus};
    use crate::policy::context::AutomationContext;
    use crate::test_utils;
    use chrono::Utc;
    use metis_common::RepoName;
    use std::sync::Arc;

    fn make_patch(status: PatchStatus, github: Option<GithubPr>) -> Patch {
        Patch::new(
            "test patch".to_string(),
            "desc".to_string(),
            String::new(),
            status,
            false,
            None,
            Vec::new(),
            RepoName::new("test", "repo").unwrap(),
            github,
        )
    }

    #[tokio::test]
    async fn skips_patch_without_github_metadata() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let new_patch = make_patch(PatchStatus::Open, None);
        let (patch_id, _) = store.add_patch(new_patch.clone()).await.unwrap();

        let payload = Arc::new(MutationPayload::Patch {
            old: None,
            new: new_patch,
        });

        let event = ServerEvent::PatchCreated {
            seq: 1,
            patch_id,
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let automation = GithubPrSyncAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        // Should be a no-op (no GitHub metadata)
        automation.execute(&ctx).await.unwrap();
    }

    #[tokio::test]
    async fn skips_closed_patch() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let github = GithubPr::new(
            "octo".to_string(),
            "repo".to_string(),
            1,
            None,
            None,
            None,
            None,
        );
        let new_patch = make_patch(PatchStatus::Closed, Some(github));
        let (patch_id, _) = store.add_patch(new_patch.clone()).await.unwrap();

        let payload = Arc::new(MutationPayload::Patch {
            old: None,
            new: new_patch,
        });

        let event = ServerEvent::PatchUpdated {
            seq: 1,
            patch_id,
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = GithubPrSyncAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        // Should be a no-op (patch is closed)
        automation.execute(&ctx).await.unwrap();
    }

    #[tokio::test]
    async fn skips_when_no_github_app_installed() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let github = GithubPr::new(
            "octo".to_string(),
            "repo".to_string(),
            1,
            Some("feature".to_string()),
            Some("main".to_string()),
            Some("https://example.com/pr/1".to_string()),
            None,
        );
        let new_patch = make_patch(PatchStatus::Open, Some(github));
        let (patch_id, _) = store.add_patch(new_patch.clone()).await.unwrap();

        let payload = Arc::new(MutationPayload::Patch {
            old: None,
            new: new_patch,
        });

        let event = ServerEvent::PatchCreated {
            seq: 1,
            patch_id,
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let automation = GithubPrSyncAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        // Should not error — just gracefully skip because no GitHub App is configured
        automation.execute(&ctx).await.unwrap();
    }
}
