use crate::{
    app::AppState,
    background::scheduler::{ScheduledWorker, WorkerOutcome},
};
use async_trait::async_trait;
use metis_common::{IssueId, SearchRepositoriesQuery, TaskId};
use octocrab::Octocrab;
use std::str::FromStr;
use tracing::{debug, info, warn};

const WORKER_NAME: &str = "cleanup_branches";

/// Maximum number of branch deletions per worker iteration across all repositories.
/// This prevents exceeding GitHub API rate limits when many stale branches accumulate.
const MAX_DELETIONS_PER_ITERATION: usize = 30;

#[derive(Clone)]
pub struct CleanupBranchesWorker {
    state: AppState,
}

impl CleanupBranchesWorker {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl ScheduledWorker for CleanupBranchesWorker {
    async fn run_iteration(&self) -> WorkerOutcome {
        info!(worker = WORKER_NAME, "worker iteration started");

        let query = SearchRepositoriesQuery::new(Some(false));
        let repositories = match self.state.store().list_repositories(&query).await {
            Ok(repos) => repos,
            Err(err) => {
                return WorkerOutcome::TransientError {
                    reason: format!("failed to list repositories: {err}"),
                };
            }
        };

        if repositories.is_empty() {
            info!(worker = WORKER_NAME, "no repositories found; worker idle");
            return WorkerOutcome::Idle;
        }

        let mut total_deleted = 0usize;
        let mut total_failed = 0usize;

        for (repo_name, repo_versioned) in &repositories {
            let remaining_budget =
                MAX_DELETIONS_PER_ITERATION.saturating_sub(total_deleted + total_failed);
            if remaining_budget == 0 {
                info!(
                    worker = WORKER_NAME,
                    limit = MAX_DELETIONS_PER_ITERATION,
                    "reached per-iteration deletion limit; deferring remaining repos to next cycle"
                );
                break;
            }

            let repo = &repo_versioned.item;
            let Some((owner, repo_short)) = parse_github_owner_repo(&repo.remote_url) else {
                debug!(
                    repo = %repo_name,
                    url = %repo.remote_url,
                    "skipping non-GitHub repository"
                );
                continue;
            };

            let Some(client) = get_installation_client(&self.state, &owner, &repo_short).await
            else {
                debug!(
                    repo = %repo_name,
                    owner = %owner,
                    repo_short = %repo_short,
                    "skipping repo: GitHub App not installed or unavailable"
                );
                continue;
            };

            match self
                .cleanup_repo_branches(&client, &owner, &repo_short, remaining_budget)
                .await
            {
                Ok(stats) => {
                    total_deleted += stats.deleted;
                    total_failed += stats.failed;
                }
                Err(err) => {
                    warn!(
                        repo = %repo_name,
                        error = %err,
                        "failed to clean up branches for repo"
                    );
                    total_failed += 1;
                }
            }
        }

        if total_deleted == 0 && total_failed == 0 {
            info!(worker = WORKER_NAME, "no stale branches found; worker idle");
            WorkerOutcome::Idle
        } else {
            info!(
                worker = WORKER_NAME,
                deleted = total_deleted,
                failed = total_failed,
                "worker iteration completed"
            );
            WorkerOutcome::Progress {
                processed: total_deleted,
                failed: total_failed,
            }
        }
    }
}

struct CleanupStats {
    deleted: usize,
    failed: usize,
}

impl CleanupBranchesWorker {
    async fn cleanup_repo_branches(
        &self,
        client: &Octocrab,
        owner: &str,
        repo: &str,
        max_deletions: usize,
    ) -> anyhow::Result<CleanupStats> {
        let refs = list_metis_refs(client, owner, repo).await?;

        if refs.is_empty() {
            return Ok(CleanupStats {
                deleted: 0,
                failed: 0,
            });
        }

        let branches: Vec<MetisBranch> = refs
            .iter()
            .filter_map(|git_ref| parse_metis_branch(&git_ref.ref_field))
            .collect();

        let mut deleted = 0usize;
        let mut failed = 0usize;

        for branch in &branches {
            if (deleted + failed) >= max_deletions {
                debug!(
                    owner = owner,
                    repo = repo,
                    limit = max_deletions,
                    "reached deletion limit for this iteration; stopping"
                );
                break;
            }

            let is_stale = self.is_branch_stale(branch).await;
            if !is_stale {
                continue;
            }

            debug!(
                owner = owner,
                repo = repo,
                branch = %branch.full_ref,
                "deleting stale branch"
            );

            if let Err(err) = delete_ref(client, owner, repo, &branch.full_ref).await {
                warn!(
                    owner = owner,
                    repo = repo,
                    branch = %branch.full_ref,
                    error = %err,
                    "failed to delete stale branch"
                );
                failed += 1;
            } else {
                deleted += 1;
            }
        }

        Ok(CleanupStats { deleted, failed })
    }

    async fn is_branch_stale(&self, branch: &MetisBranch) -> bool {
        match &branch.id_kind {
            MetisIdKind::Issue(id) => {
                match self.state.store().get_issue(id, true).await {
                    Ok(versioned) => versioned.item.deleted,
                    Err(_) => false, // Unknown issue -- do not delete
                }
            }
            MetisIdKind::Task(id) => {
                match self.state.store().get_task(id, true).await {
                    Ok(versioned) => versioned.item.deleted,
                    Err(_) => false, // Unknown task -- do not delete
                }
            }
        }
    }
}

/// A parsed metis tracking branch reference.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MetisBranch {
    full_ref: String,
    id_kind: MetisIdKind,
    suffix: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MetisIdKind {
    Issue(IssueId),
    Task(TaskId),
}

/// Minimal representation of a Git reference from the GitHub API.
#[derive(Debug, Clone, serde::Deserialize)]
struct GitRef {
    #[serde(rename = "ref")]
    ref_field: String,
}

/// Parse a GitHub remote URL to extract owner and repo name.
///
/// Supports HTTPS (https://github.com/owner/repo.git) and
/// SSH (git@github.com:owner/repo.git) formats.
fn parse_github_owner_repo(remote_url: &str) -> Option<(String, String)> {
    // HTTPS: https://github.com/owner/repo.git or https://github.com/owner/repo
    if let Some(path) = remote_url
        .strip_prefix("https://github.com/")
        .or_else(|| remote_url.strip_prefix("http://github.com/"))
    {
        let path = path.trim_end_matches('/').trim_end_matches(".git");
        let (owner, repo) = path.split_once('/')?;
        if owner.is_empty() || repo.is_empty() || repo.contains('/') {
            return None;
        }
        return Some((owner.to_string(), repo.to_string()));
    }

    // SSH: git@github.com:owner/repo.git
    if let Some(path) = remote_url.strip_prefix("git@github.com:") {
        let path = path.trim_end_matches('/').trim_end_matches(".git");
        let (owner, repo) = path.split_once('/')?;
        if owner.is_empty() || repo.is_empty() || repo.contains('/') {
            return None;
        }
        return Some((owner.to_string(), repo.to_string()));
    }

    None
}

/// Parse a Git ref string like "refs/heads/metis/i-abcdef/head" into a MetisBranch.
fn parse_metis_branch(ref_name: &str) -> Option<MetisBranch> {
    let branch_path = ref_name.strip_prefix("refs/heads/")?;

    // Expected pattern: metis/<id>/<suffix>
    let rest = branch_path.strip_prefix("metis/")?;

    let slash_pos = rest.find('/')?;
    let id_str = &rest[..slash_pos];
    let suffix = &rest[slash_pos + 1..];

    if suffix.is_empty() {
        return None;
    }

    let id_kind = if id_str.starts_with(IssueId::prefix()) {
        let id = IssueId::from_str(id_str).ok()?;
        MetisIdKind::Issue(id)
    } else if id_str.starts_with(TaskId::prefix()) {
        let id = TaskId::from_str(id_str).ok()?;
        MetisIdKind::Task(id)
    } else {
        return None;
    };

    Some(MetisBranch {
        full_ref: ref_name.to_string(),
        id_kind,
        suffix: suffix.to_string(),
    })
}

/// Get a GitHub installation client for the given owner/repo.
async fn get_installation_client(state: &AppState, owner: &str, repo: &str) -> Option<Octocrab> {
    let app_client = state.github_app.as_ref()?;

    let installation = match app_client
        .apps()
        .get_repository_installation(owner, repo)
        .await
    {
        Ok(installation) => installation,
        Err(err) => {
            debug!(
                owner = owner,
                repo = repo,
                error = %err,
                "failed to lookup GitHub App installation"
            );
            return None;
        }
    };

    match app_client.installation_and_token(installation.id).await {
        Ok((client, _token)) => Some(client),
        Err(err) => {
            debug!(
                owner = owner,
                repo = repo,
                error = %err,
                "failed to fetch GitHub App installation token"
            );
            None
        }
    }
}

/// List all Git references matching the metis/ prefix for a repository.
async fn list_metis_refs(
    client: &Octocrab,
    owner: &str,
    repo: &str,
) -> anyhow::Result<Vec<GitRef>> {
    let url = format!("/repos/{owner}/{repo}/git/matching-refs/heads/metis/");
    let refs: Vec<GitRef> = client.get(url, None::<&()>).await?;
    Ok(refs)
}

/// Delete a single Git reference from a repository.
async fn delete_ref(
    client: &Octocrab,
    owner: &str,
    repo: &str,
    full_ref: &str,
) -> anyhow::Result<()> {
    // The API expects the ref without the "refs/" prefix in some cases,
    // but the DELETE endpoint uses the full ref path after /git/refs/
    let ref_path = full_ref.strip_prefix("refs/").unwrap_or(full_ref);
    let url = format!("/repos/{owner}/{repo}/git/refs/{ref_path}");
    let _: serde_json::Value = client.delete(url, None::<&()>).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::actors::ActorRef;
    use crate::domain::users::Username;
    use httpmock::prelude::*;
    use serde_json::json;

    #[test]
    fn parse_github_owner_repo_https_with_git_suffix() {
        let result = parse_github_owner_repo("https://github.com/dourolabs/metis.git");
        assert_eq!(result, Some(("dourolabs".to_string(), "metis".to_string())));
    }

    #[test]
    fn parse_github_owner_repo_https_without_git_suffix() {
        let result = parse_github_owner_repo("https://github.com/dourolabs/metis");
        assert_eq!(result, Some(("dourolabs".to_string(), "metis".to_string())));
    }

    #[test]
    fn parse_github_owner_repo_ssh() {
        let result = parse_github_owner_repo("git@github.com:dourolabs/metis.git");
        assert_eq!(result, Some(("dourolabs".to_string(), "metis".to_string())));
    }

    #[test]
    fn parse_github_owner_repo_non_github() {
        assert_eq!(
            parse_github_owner_repo("https://gitlab.com/org/repo.git"),
            None
        );
    }

    #[test]
    fn parse_github_owner_repo_empty_segments() {
        assert_eq!(
            parse_github_owner_repo("https://github.com//repo.git"),
            None
        );
        assert_eq!(parse_github_owner_repo("https://github.com/owner/"), None);
    }

    #[test]
    fn parse_metis_branch_issue_head() {
        let branch = parse_metis_branch("refs/heads/metis/i-abcdef/head");
        assert!(branch.is_some());
        let branch = branch.unwrap();
        assert_eq!(branch.suffix, "head");
        assert!(matches!(branch.id_kind, MetisIdKind::Issue(_)));
        assert_eq!(branch.full_ref, "refs/heads/metis/i-abcdef/head");
    }

    #[test]
    fn parse_metis_branch_issue_base() {
        let branch = parse_metis_branch("refs/heads/metis/i-abcdef/base");
        assert!(branch.is_some());
        let branch = branch.unwrap();
        assert_eq!(branch.suffix, "base");
        assert!(matches!(branch.id_kind, MetisIdKind::Issue(_)));
    }

    #[test]
    fn parse_metis_branch_task_head() {
        let branch = parse_metis_branch("refs/heads/metis/t-xyzabc/head");
        assert!(branch.is_some());
        let branch = branch.unwrap();
        assert_eq!(branch.suffix, "head");
        assert!(matches!(branch.id_kind, MetisIdKind::Task(_)));
    }

    #[test]
    fn parse_metis_branch_task_base() {
        let branch = parse_metis_branch("refs/heads/metis/t-xyzabc/base");
        assert!(branch.is_some());
        let branch = branch.unwrap();
        assert_eq!(branch.suffix, "base");
        assert!(matches!(branch.id_kind, MetisIdKind::Task(_)));
    }

    #[test]
    fn parse_metis_branch_non_metis_ref() {
        assert!(parse_metis_branch("refs/heads/main").is_none());
        assert!(parse_metis_branch("refs/heads/feature/foo").is_none());
    }

    #[test]
    fn parse_metis_branch_invalid_id_prefix() {
        assert!(parse_metis_branch("refs/heads/metis/p-abcdef/head").is_none());
        assert!(parse_metis_branch("refs/heads/metis/d-abcdef/head").is_none());
    }

    #[test]
    fn parse_metis_branch_missing_suffix() {
        assert!(parse_metis_branch("refs/heads/metis/i-abcdef").is_none());
    }

    #[test]
    fn parse_metis_branch_invalid_id_format() {
        // IDs must have 4-12 lowercase alpha chars after prefix
        assert!(parse_metis_branch("refs/heads/metis/i-ab/head").is_none());
        assert!(parse_metis_branch("refs/heads/metis/i-123456/head").is_none());
    }

    #[tokio::test]
    async fn worker_returns_idle_without_repositories() {
        let state = crate::test_utils::test_state();
        let worker = CleanupBranchesWorker::new(state);

        let outcome = worker.run_iteration().await;
        assert_eq!(outcome, WorkerOutcome::Idle);
    }

    #[tokio::test]
    async fn worker_returns_transient_error_when_store_fails() {
        let handles = crate::test_utils::test_state_with_store(std::sync::Arc::new(
            crate::test_utils::FailingStore,
        ));
        let worker = CleanupBranchesWorker::new(handles.state);

        let outcome = worker.run_iteration().await;
        assert!(matches!(outcome, WorkerOutcome::TransientError { .. }));
    }

    #[tokio::test]
    async fn worker_skips_non_github_repos() -> anyhow::Result<()> {
        let handles = crate::test_utils::test_state_handles();
        crate::test_utils::add_repository(
            &handles.state,
            metis_common::RepoName::from_str("org/repo")?,
            metis_common::Repository::new(
                "https://gitlab.com/org/repo.git".to_string(),
                None,
                None,
                None,
            ),
        )
        .await?;

        let worker = CleanupBranchesWorker::new(handles.state);
        let outcome = worker.run_iteration().await;

        // No GitHub repos to process, should be idle
        assert_eq!(outcome, WorkerOutcome::Idle);
        Ok(())
    }

    #[tokio::test]
    async fn is_branch_stale_returns_false_for_unknown_issue() {
        let state = crate::test_utils::test_state();
        let worker = CleanupBranchesWorker::new(state);

        let branch = MetisBranch {
            full_ref: "refs/heads/metis/i-nonexist/head".to_string(),
            id_kind: MetisIdKind::Issue(IssueId::new()),
            suffix: "head".to_string(),
        };

        assert!(!worker.is_branch_stale(&branch).await);
    }

    #[tokio::test]
    async fn is_branch_stale_returns_false_for_unknown_task() {
        let state = crate::test_utils::test_state();
        let worker = CleanupBranchesWorker::new(state);

        let branch = MetisBranch {
            full_ref: "refs/heads/metis/t-nonexist/head".to_string(),
            id_kind: MetisIdKind::Task(TaskId::new()),
            suffix: "head".to_string(),
        };

        assert!(!worker.is_branch_stale(&branch).await);
    }

    #[tokio::test]
    async fn is_branch_stale_returns_false_for_existing_issue() {
        let handles = crate::test_utils::test_state_handles();
        let issue = crate::domain::issues::Issue::new(
            crate::domain::issues::IssueType::Task,
            "Test Title".to_string(),
            "test issue".to_string(),
            crate::domain::users::Username::from("creator"),
            String::new(),
            crate::domain::issues::IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        let (issue_id, _) = handles
            .store
            .add_issue(issue, &ActorRef::test())
            .await
            .unwrap();

        let worker = CleanupBranchesWorker::new(handles.state);
        let branch = MetisBranch {
            full_ref: format!("refs/heads/metis/{issue_id}/head"),
            id_kind: MetisIdKind::Issue(issue_id),
            suffix: "head".to_string(),
        };

        assert!(!worker.is_branch_stale(&branch).await);
    }

    #[tokio::test]
    async fn is_branch_stale_returns_false_for_existing_task() {
        let handles = crate::test_utils::test_state_handles();
        let task = crate::store::Task::new(
            "test task".to_string(),
            crate::domain::jobs::BundleSpec::None,
            None,
            Username::from("test-creator"),
            None,
            None,
            std::collections::HashMap::new(),
            None,
            None,
            crate::store::Status::Created,
            None,
            None,
        );
        let (task_id, _) = handles
            .store
            .add_task(task, chrono::Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let worker = CleanupBranchesWorker::new(handles.state);
        let branch = MetisBranch {
            full_ref: format!("refs/heads/metis/{task_id}/head"),
            id_kind: MetisIdKind::Task(task_id),
            suffix: "head".to_string(),
        };

        assert!(!worker.is_branch_stale(&branch).await);
    }

    #[tokio::test]
    async fn is_branch_stale_returns_true_for_deleted_issue() {
        let handles = crate::test_utils::test_state_handles();
        let issue = crate::domain::issues::Issue::new(
            crate::domain::issues::IssueType::Task,
            "Test Title".to_string(),
            "deleted issue".to_string(),
            crate::domain::users::Username::from("creator"),
            String::new(),
            crate::domain::issues::IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        let (issue_id, _) = handles
            .store
            .add_issue(issue, &ActorRef::test())
            .await
            .unwrap();
        handles
            .store
            .delete_issue(&issue_id, &ActorRef::test())
            .await
            .unwrap();

        let worker = CleanupBranchesWorker::new(handles.state);
        let branch = MetisBranch {
            full_ref: format!("refs/heads/metis/{issue_id}/head"),
            id_kind: MetisIdKind::Issue(issue_id),
            suffix: "head".to_string(),
        };

        assert!(worker.is_branch_stale(&branch).await);
    }

    #[tokio::test]
    async fn is_branch_stale_returns_true_for_deleted_task() {
        let handles = crate::test_utils::test_state_handles();
        let task = crate::store::Task::new(
            "deleted task".to_string(),
            crate::domain::jobs::BundleSpec::None,
            None,
            Username::from("test-creator"),
            None,
            None,
            std::collections::HashMap::new(),
            None,
            None,
            crate::store::Status::Created,
            None,
            None,
        );
        let (task_id, _) = handles
            .store
            .add_task(task, chrono::Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        handles
            .store
            .delete_task(&task_id, &ActorRef::test())
            .await
            .unwrap();

        let worker = CleanupBranchesWorker::new(handles.state);
        let branch = MetisBranch {
            full_ref: format!("refs/heads/metis/{task_id}/head"),
            id_kind: MetisIdKind::Task(task_id),
            suffix: "head".to_string(),
        };

        assert!(worker.is_branch_stale(&branch).await);
    }

    /// Helper: create an issue in the store, then soft-delete it so it appears stale.
    /// Returns the generated issue ID.
    async fn create_deleted_issue(store: &dyn crate::store::Store, label: &str) -> IssueId {
        let issue = crate::domain::issues::Issue::new(
            crate::domain::issues::IssueType::Task,
            "Test Title".to_string(),
            label.to_string(),
            Username::from("creator"),
            String::new(),
            crate::domain::issues::IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        let (issue_id, _) = store.add_issue(issue, &ActorRef::test()).await.unwrap();
        store
            .delete_issue(&issue_id, &ActorRef::test())
            .await
            .unwrap();
        issue_id
    }

    #[tokio::test]
    async fn cleanup_repo_branches_stops_after_budget_exhausted_by_failures() {
        let handles = crate::test_utils::test_state_handles();

        // Create 5 deleted (stale) issues.
        let mut issue_ids = Vec::new();
        for i in 0..5 {
            let id = create_deleted_issue(handles.store.as_ref(), &format!("stale-{i}")).await;
            issue_ids.push(id);
        }

        // Stand up an httpmock server and mock the list-refs endpoint.
        let server = MockServer::start();
        let refs_json: Vec<serde_json::Value> = issue_ids
            .iter()
            .map(|id| json!({"ref": format!("refs/heads/metis/{id}/head")}))
            .collect();
        server.mock(|when, then| {
            when.method(GET)
                .path("/repos/testowner/testrepo/git/matching-refs/heads/metis/");
            then.status(200).json_body(json!(refs_json));
        });

        // Branch 0: delete succeeds
        let ref_path_0 = format!("heads/metis/{}/head", issue_ids[0]);
        server.mock(|when, then| {
            when.method(DELETE)
                .path(format!("/repos/testowner/testrepo/git/refs/{ref_path_0}"));
            then.status(200).json_body(json!({}));
        });
        // Branch 1: delete fails (422)
        let ref_path_1 = format!("heads/metis/{}/head", issue_ids[1]);
        server.mock(|when, then| {
            when.method(DELETE)
                .path(format!("/repos/testowner/testrepo/git/refs/{ref_path_1}"));
            then.status(422)
                .json_body(json!({"message": "Reference does not exist"}));
        });
        // Branch 2: delete succeeds
        let ref_path_2 = format!("heads/metis/{}/head", issue_ids[2]);
        server.mock(|when, then| {
            when.method(DELETE)
                .path(format!("/repos/testowner/testrepo/git/refs/{ref_path_2}"));
            then.status(200).json_body(json!({}));
        });
        // Branches 3 and 4 should never be reached because budget is exhausted.

        // Build a plain Octocrab client pointed at the mock server.
        let client = Octocrab::builder()
            .base_uri(server.base_url())
            .unwrap()
            .build()
            .unwrap();

        let worker = CleanupBranchesWorker::new(handles.state);
        let stats = worker
            .cleanup_repo_branches(&client, "testowner", "testrepo", 3)
            .await
            .unwrap();

        assert_eq!(stats.deleted, 2);
        assert_eq!(stats.failed, 1);
        assert_eq!(stats.deleted + stats.failed, 3);
    }

    #[tokio::test]
    async fn run_iteration_counts_failed_deletions_against_budget() {
        // Build a mock GitHub App so get_installation_client() succeeds.
        let (server, github_app) = crate::test_utils::GitHubMockBuilder::new()
            .with_installation("testowner", "testrepo")
            .build()
            .unwrap();

        let handles = crate::test_utils::test_state_with_github_app(github_app);

        // Register a GitHub-hosted repository in the store.
        crate::test_utils::add_repository(
            &handles.state,
            metis_common::RepoName::from_str("testowner/testrepo").unwrap(),
            metis_common::Repository::new(
                "https://github.com/testowner/testrepo.git".to_string(),
                None,
                None,
                None,
            ),
        )
        .await
        .unwrap();

        // Create and delete 4 issues so their branches are stale.
        let mut issue_ids = Vec::new();
        for i in 0..4 {
            let id = create_deleted_issue(handles.store.as_ref(), &format!("iter-{i}")).await;
            issue_ids.push(id);
        }

        // Mock the list-refs endpoint on the same mock server.
        let refs_json: Vec<serde_json::Value> = issue_ids
            .iter()
            .map(|id| json!({"ref": format!("refs/heads/metis/{id}/head")}))
            .collect();
        server.mock(|when, then| {
            when.method(GET)
                .path("/repos/testowner/testrepo/git/matching-refs/heads/metis/");
            then.status(200).json_body(json!(refs_json));
        });

        // Mock delete-ref calls: even-indexed succeed, odd-indexed fail.
        for (i, id) in issue_ids.iter().enumerate() {
            let ref_path = format!("heads/metis/{id}/head");
            if i % 2 == 0 {
                server.mock(|when, then| {
                    when.method(DELETE)
                        .path(format!("/repos/testowner/testrepo/git/refs/{ref_path}"));
                    then.status(200).json_body(json!({}));
                });
            } else {
                server.mock(|when, then| {
                    when.method(DELETE)
                        .path(format!("/repos/testowner/testrepo/git/refs/{ref_path}"));
                    then.status(422)
                        .json_body(json!({"message": "Reference does not exist"}));
                });
            }
        }

        let worker = CleanupBranchesWorker::new(handles.state);
        let outcome = worker.run_iteration().await;

        match outcome {
            WorkerOutcome::Progress { processed, failed } => {
                assert_eq!(processed, 2);
                assert_eq!(failed, 2);
                assert!(processed + failed <= MAX_DELETIONS_PER_ITERATION);
            }
            other => panic!("expected WorkerOutcome::Progress, got {other:?}"),
        }
    }
}
