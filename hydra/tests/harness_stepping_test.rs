mod harness;

use anyhow::Result;
use hydra_common::issues::{Issue, IssueStatus, IssueType, SessionSettings, UpsertIssueRequest};
use hydra_common::users::Username;
use std::str::FromStr;

/// Helper: register an agent queue in the harness and create an issue
/// assigned to that agent with the given repo.
async fn create_spawnable_issue(
    harness: &harness::TestHarness,
    agent_name: &str,
    repo_name: &str,
    description: &str,
) -> Result<hydra_common::IssueId> {
    let repo = hydra_common::RepoName::from_str(repo_name)?;
    let mut job_settings = SessionSettings::default();
    job_settings.repo_name = Some(repo);

    let issue = Issue::new(
        IssueType::Task,
        "Test Title".to_string(),
        description.to_string(),
        Username::from("default"),
        String::new(),
        IssueStatus::Open,
        Some(agent_name.to_string()),
        Some(job_settings),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        false,
    );
    let request = UpsertIssueRequest::new(issue, None);
    let response = harness
        .default_user()
        .client()
        .create_issue(&request)
        .await?;
    Ok(response.issue_id)
}

/// When no agents are configured, no sessions are spawned automatically.
#[tokio::test]
async fn auto_spawn_no_agents_returns_empty() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("test-org/test-repo")
        .build()
        .await?;

    harness.step_pending_jobs().await?;
    let all_sessions = harness.state().list_sessions().await?;
    assert!(
        all_sessions.is_empty(),
        "no agents configured, should create no sessions"
    );

    Ok(())
}

/// When no ready issues exist, no sessions are spawned automatically.
#[tokio::test]
async fn auto_spawn_no_ready_issues_returns_empty() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("test-org/test-repo")
        .with_agent("swe", "You are a software engineer")
        .build()
        .await?;

    harness.step_pending_jobs().await?;
    let all_sessions = harness.state().list_sessions().await?;
    assert!(
        all_sessions.is_empty(),
        "no issues exist, should create no sessions"
    );

    Ok(())
}

/// Sessions are spawned automatically when an issue is created with an agent.
#[tokio::test]
async fn auto_spawn_creates_session_for_ready_issue() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/app")
        .with_agent("swe", "You are a software engineer")
        .build()
        .await?;

    let issue_id = create_spawnable_issue(&harness, "swe", "acme/app", "implement feature").await?;

    harness.step_pending_jobs().await?;
    let sessions = harness.list_sessions_for_issue(&issue_id, vec![]).await?;
    assert_eq!(
        sessions.len(),
        1,
        "spawn_sessions automation should create exactly one session for the ready issue"
    );

    Ok(())
}

/// step_pending_jobs waits for the automation to transition created sessions to pending.
#[tokio::test]
async fn step_pending_jobs_processes_created_sessions() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/app")
        .with_agent("swe", "You are a software engineer")
        .build()
        .await?;

    let issue_id = create_spawnable_issue(&harness, "swe", "acme/app", "process test").await?;

    harness.step_pending_jobs().await?;
    let task_ids = harness.list_sessions_for_issue(&issue_id, vec![]).await?;
    assert_eq!(task_ids.len(), 1);

    Ok(())
}

/// Automations spawn and process sessions for a ready issue.
#[tokio::test]
async fn auto_spawn_creates_and_processes_sessions() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/app")
        .with_agent("swe", "You are a software engineer")
        .build()
        .await?;

    let issue_id = create_spawnable_issue(&harness, "swe", "acme/app", "schedule test").await?;

    harness.step_pending_jobs().await?;
    let task_ids = harness.list_sessions_for_issue(&issue_id, vec![]).await?;
    assert_eq!(
        task_ids.len(),
        1,
        "automation should create and process exactly one session for the ready issue"
    );

    Ok(())
}

/// step_github_sync runs without error when no patches exist.
#[tokio::test]
async fn step_github_sync_idle_without_patches() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/app")
        .with_github()
        .build()
        .await?;

    // Should not error — just idle.
    harness.step_github_sync().await?;

    Ok(())
}

/// step_monitor_jobs runs without error when no active tasks exist.
#[tokio::test]
async fn step_monitor_jobs_idle_without_tasks() -> Result<()> {
    let harness = harness::TestHarness::new().await?;

    // Should not error — just idle.
    harness.step_monitor_jobs().await?;

    Ok(())
}

/// Stepping is deterministic: same sequence of steps produces same result.
#[tokio::test]
async fn stepping_is_deterministic() -> Result<()> {
    // Run the same scenario twice and verify identical results.
    async fn run_scenario() -> Result<usize> {
        let harness = harness::TestHarness::builder()
            .with_repo("acme/deterministic")
            .with_agent("swe", "You are a software engineer")
            .build()
            .await?;
        let issue_id =
            create_spawnable_issue(&harness, "swe", "acme/deterministic", "determinism test")
                .await?;
        harness.step_pending_jobs().await?;
        let sessions = harness.list_sessions_for_issue(&issue_id, vec![]).await?;
        Ok(sessions.len())
    }

    let result1 = run_scenario().await?;
    let result2 = run_scenario().await?;

    assert_eq!(
        result1, result2,
        "same scenario should produce same number of sessions"
    );
    // Both should produce exactly 1 session.
    assert_eq!(result1, 1);
    assert_eq!(result2, 1);

    Ok(())
}

/// The automation does not create duplicate sessions for the same issue.
#[tokio::test]
async fn auto_spawn_no_duplicate_sessions() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/dedup")
        .with_agent("swe", "You are a software engineer")
        .build()
        .await?;

    let issue_id = create_spawnable_issue(&harness, "swe", "acme/dedup", "dedup test").await?;

    // First: should create exactly one session for the issue.
    harness.step_pending_jobs().await?;
    let first = harness.list_sessions_for_issue(&issue_id, vec![]).await?;
    assert_eq!(first.len(), 1);

    // Second: issue already has an active session, total should still be 1.
    harness.step_pending_jobs().await?;
    let second = harness.list_sessions_for_issue(&issue_id, vec![]).await?;
    assert_eq!(
        second.len(),
        1,
        "automation should not create duplicate sessions for the same issue"
    );

    Ok(())
}
