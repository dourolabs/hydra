use async_trait::async_trait;
use octocrab::Octocrab;
use tracing::{info, warn};

use crate::app::AppState;
use crate::app::event_bus::{EventType, MutationPayload, ServerEvent};
use crate::domain::actors::Actor;
use crate::domain::patches::{GithubPr, Patch};
use crate::policy::context::AutomationContext;
use crate::policy::{Automation, AutomationError, EventFilter};
use metis_common::PatchId;

/// Synchronizes patches with GitHub pull requests.
///
/// When a patch is created or updated with a `branch_name`, this automation
/// creates or updates the corresponding GitHub pull request.
///
/// - **With actor context** (authenticated request): uses the actor's personal
///   GitHub token so the PR is attributed to the user.
/// - **Without actor context** (background/cascading event): uses the GitHub
///   App installation client as a fallback for PR updates only; PR creation
///   requires a personal token and is skipped.
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
            event_types: vec![EventType::PatchCreated, EventType::PatchUpdated],
        }
    }

    async fn execute(&self, ctx: &AutomationContext<'_>) -> Result<(), AutomationError> {
        let (patch_id, new_patch) = match ctx.event {
            ServerEvent::PatchCreated {
                patch_id, payload, ..
            }
            | ServerEvent::PatchUpdated {
                patch_id, payload, ..
            } => {
                let MutationPayload::Patch { new, .. } = payload.as_ref() else {
                    return Ok(());
                };
                (patch_id, new)
            }
            _ => return Ok(()),
        };

        let Some(head_ref) = new_patch.branch_name.as_deref() else {
            return Ok(());
        };

        if new_patch.github.is_some() {
            update_existing_pr(ctx, patch_id, new_patch, head_ref).await
        } else {
            create_new_pr(ctx, patch_id, new_patch, head_ref).await
        }
    }
}

/// Update an existing GitHub pull request's title and description.
async fn update_existing_pr(
    ctx: &AutomationContext<'_>,
    patch_id: &PatchId,
    patch: &Patch,
    _head_ref: &str,
) -> Result<(), AutomationError> {
    let github = patch.github.as_ref().expect("checked by caller");
    let (owner, repo) = (&github.owner, &github.repo);

    // Prefer the actor's personal token (preserves authorship); fall back to
    // the GitHub App installation client when no actor is available.
    let client = if let Some(actor) = ctx.actor {
        match github_user_client(ctx.app_state, actor).await {
            Ok(c) => c,
            Err(e) => {
                warn!(
                    patch_id = %patch_id,
                    error = %e,
                    "failed to build user client for PR update, trying app client"
                );
                match select_github_installation_client(ctx.app_state, github).await? {
                    Some(c) => c,
                    None => return Ok(()),
                }
            }
        }
    } else {
        match select_github_installation_client(ctx.app_state, github).await? {
            Some(c) => c,
            None => return Ok(()),
        }
    };

    let pr = client
        .pulls(owner, repo)
        .update(github.number)
        .title(patch.title.clone())
        .body(patch.description.clone())
        .send()
        .await
        .map_err(|e| {
            AutomationError::Other(anyhow::anyhow!(
                "failed to update PR {owner}/{repo}#{}: {e}",
                github.number
            ))
        })?;

    // Persist updated GitHub metadata back to the patch.
    let mut updated_github = github.clone();
    updated_github.head_ref = Some(pr.head.ref_field.clone());
    updated_github.base_ref = Some(pr.base.ref_field.clone());
    updated_github.url = pr.html_url.as_ref().map(ToString::to_string);

    let mut updated_patch = patch.clone();
    updated_patch.github = Some(updated_github);

    let request = metis_common::api::v1::patches::UpsertPatchRequest::new(updated_patch.into());
    ctx.app_state
        .upsert_patch(None, Some(patch_id.clone()), request)
        .await
        .map_err(|e| {
            AutomationError::Other(anyhow::anyhow!(
                "failed to persist GitHub metadata for patch {patch_id}: {e}"
            ))
        })?;

    info!(patch_id = %patch_id, "updated GitHub PR");
    Ok(())
}

/// Create a new GitHub pull request for a patch.
///
/// Requires an actor with a personal GitHub token; skips silently when no actor
/// is available (the poller or a future event with an actor will handle it).
async fn create_new_pr(
    ctx: &AutomationContext<'_>,
    patch_id: &PatchId,
    patch: &Patch,
    head_ref: &str,
) -> Result<(), AutomationError> {
    let Some(actor) = ctx.actor else {
        info!(
            patch_id = %patch_id,
            "skipping PR creation: no actor context available"
        );
        return Ok(());
    };

    let (owner, repo) = (
        patch.service_repo_name.organization.clone(),
        patch.service_repo_name.repo.clone(),
    );

    let client = github_user_client(ctx.app_state, actor)
        .await
        .map_err(|e| {
            AutomationError::Other(anyhow::anyhow!(
                "failed to build GitHub client for PR creation: {e}"
            ))
        })?;

    let base_ref = resolve_base_ref(ctx.app_state, patch).await?;

    let pr = client
        .pulls(&owner, &repo)
        .create(patch.title.clone(), head_ref, base_ref)
        .body(patch.description.clone())
        .send()
        .await
        .map_err(|e| {
            AutomationError::Other(anyhow::anyhow!(
                "failed to create PR for {owner}/{repo}: {e}"
            ))
        })?;

    let github = GithubPr::new(
        owner,
        repo,
        pr.number,
        Some(pr.head.ref_field.clone()),
        Some(pr.base.ref_field.clone()),
        pr.html_url.as_ref().map(ToString::to_string),
        None,
    );

    let mut updated_patch = patch.clone();
    updated_patch.github = Some(github);

    let request = metis_common::api::v1::patches::UpsertPatchRequest::new(updated_patch.into());
    ctx.app_state
        .upsert_patch(None, Some(patch_id.clone()), request)
        .await
        .map_err(|e| {
            AutomationError::Other(anyhow::anyhow!(
                "failed to persist GitHub PR metadata for patch {patch_id}: {e}"
            ))
        })?;

    info!(patch_id = %patch_id, "created GitHub PR");
    Ok(())
}

/// Resolve the base branch for a new PR, using the patch's GitHub metadata or
/// falling back to the repository's default branch.
async fn resolve_base_ref(state: &AppState, patch: &Patch) -> Result<String, AutomationError> {
    // Check if the patch already specifies a base ref via github metadata.
    if let Some(base) = patch
        .github
        .as_ref()
        .and_then(|g| g.base_ref.as_ref())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return Ok(base);
    }

    // Fall back to the repository's configured default branch.
    let repository = state
        .repository_from_store(&patch.service_repo_name)
        .await
        .map_err(|e| {
            AutomationError::Other(anyhow::anyhow!(
                "failed to load repository '{}' for base ref: {e}",
                patch.service_repo_name,
            ))
        })?;

    repository
        .default_branch
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            AutomationError::Other(anyhow::anyhow!(
                "no base ref available for repository '{}'",
                patch.service_repo_name,
            ))
        })
}

/// Build an authenticated Octocrab client using the actor's personal GitHub
/// token.
async fn github_user_client(state: &AppState, actor: &Actor) -> Result<Octocrab, anyhow::Error> {
    let token = actor.get_github_token(state).await.map_err(|e| {
        anyhow::anyhow!("failed to load GitHub token for {}: {:?}", actor.name(), e)
    })?;

    Octocrab::builder()
        .base_uri(state.config.github_app.api_base_url().to_string())
        .map_err(|e| anyhow::anyhow!("invalid GitHub API base URL: {e}"))?
        .personal_token(token.github_token)
        .build()
        .map_err(|e| anyhow::anyhow!("failed to build Octocrab client: {e}"))
}

/// Get a GitHub App installation client for the given repository.
async fn select_github_installation_client(
    state: &AppState,
    github: &GithubPr,
) -> Result<Option<Octocrab>, AutomationError> {
    let Some(app_client) = state.github_app.as_ref() else {
        return Ok(None);
    };

    let installation = match app_client
        .apps()
        .get_repository_installation(&github.owner, &github.repo)
        .await
    {
        Ok(installation) => installation,
        Err(err) => {
            warn!(
                owner = %github.owner,
                repo = %github.repo,
                error = %err,
                "failed to lookup GitHub App installation"
            );
            return Ok(None);
        }
    };

    let (installation_client, _token) =
        match app_client.installation_and_token(installation.id).await {
            Ok(result) => result,
            Err(err) => {
                warn!(
                    owner = %github.owner,
                    repo = %github.repo,
                    installation_id = %installation.id,
                    error = %err,
                    "failed to fetch GitHub App installation token"
                );
                return Ok(None);
            }
        };

    Ok(Some(installation_client))
}
