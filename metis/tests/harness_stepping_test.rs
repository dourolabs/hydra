mod harness;

use anyhow::Result;
use chrono::Utc;
use metis_common::issues::{Issue, IssueStatus, IssueType, JobSettings, UpsertIssueRequest};
use metis_common::users::Username;
use metis_server::{background::spawner::AgentQueue, config::AgentQueueConfig};
use std::str::FromStr;
use std::sync::Arc;

/// Helper: register an agent queue in the harness and create an issue
/// assigned to that agent with the given repo.
async fn create_spawnable_issue(
    harness: &harness::TestHarness,
    agent_name: &str,
    repo_name: &str,
    description: &str,
) -> Result<metis_common::IssueId> {
    let repo = metis_common::RepoName::from_str(repo_name)?;
    let mut job_settings = JobSettings::default();
    job_settings.repo_name = Some(repo);

    let issue = Issue::new(
        IssueType::Task,
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
        Utc::now(),
    );
    let request = UpsertIssueRequest::new(issue, None);
    let response = harness
        .default_user()
        .client()
        .create_issue(&request)
        .await?;
    Ok(response.issue_id)
}

/// Helper: register an agent queue in the harness.
async fn register_agent(harness: &harness::TestHarness, name: &str) {
    let config = AgentQueueConfig {
        name: name.to_string(),
        prompt: format!("test prompt for {name}"),
        max_tries: 3,
        max_simultaneous: 10,
    };
    let mut agents = harness.agents().write().await;
    agents.push(Arc::new(AgentQueue::from_config(&config)));
}

/// step_spawner with no agents configured returns empty vec.
#[tokio::test]
async fn step_spawner_no_agents_returns_empty() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("test-org/test-repo")
        .build()
        .await?;

    let created = harness.step_spawner().await?;
    assert!(
        created.is_empty(),
        "no agents configured, should create no tasks"
    );

    Ok(())
}

/// step_spawner with no ready issues returns empty vec.
#[tokio::test]
async fn step_spawner_no_ready_issues_returns_empty() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("test-org/test-repo")
        .build()
        .await?;
    register_agent(&harness, "swe").await;

    let created = harness.step_spawner().await?;
    assert!(
        created.is_empty(),
        "no issues exist, should create no tasks"
    );

    Ok(())
}

/// step_spawner creates a task when there is a ready issue assigned to an agent.
#[tokio::test]
async fn step_spawner_creates_task_for_ready_issue() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/app")
        .build()
        .await?;
    register_agent(&harness, "swe").await;

    let _issue_id =
        create_spawnable_issue(&harness, "swe", "acme/app", "implement feature").await?;

    let created = harness.step_spawner().await?;
    assert_eq!(
        created.len(),
        1,
        "spawner should create exactly one task for the ready issue"
    );

    Ok(())
}

/// step_pending_jobs transitions created tasks to pending.
#[tokio::test]
async fn step_pending_jobs_processes_created_tasks() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/app")
        .build()
        .await?;
    register_agent(&harness, "swe").await;

    let _issue_id = create_spawnable_issue(&harness, "swe", "acme/app", "process test").await?;

    // Create tasks via spawner.
    let task_ids = harness.step_spawner().await?;
    assert_eq!(task_ids.len(), 1);

    // Process pending jobs.
    let processed = harness.step_pending_jobs().await?;
    assert_eq!(processed.len(), 1, "should process the one created task");

    Ok(())
}

/// step_schedule combines spawner + pending jobs in one call.
#[tokio::test]
async fn step_schedule_creates_and_processes_tasks() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/app")
        .build()
        .await?;
    register_agent(&harness, "swe").await;

    let _issue_id = create_spawnable_issue(&harness, "swe", "acme/app", "schedule test").await?;

    let task_ids = harness.step_schedule().await?;
    assert_eq!(
        task_ids.len(),
        1,
        "step_schedule should return the task created by the spawner"
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
    async fn run_scenario() -> Result<Vec<metis_common::TaskId>> {
        let harness = harness::TestHarness::builder()
            .with_repo("acme/deterministic")
            .build()
            .await?;
        register_agent(&harness, "swe").await;
        create_spawnable_issue(&harness, "swe", "acme/deterministic", "determinism test").await?;
        harness.step_schedule().await
    }

    let result1 = run_scenario().await?;
    let result2 = run_scenario().await?;

    assert_eq!(
        result1.len(),
        result2.len(),
        "same scenario should produce same number of tasks"
    );
    // Both should produce exactly 1 task.
    assert_eq!(result1.len(), 1);
    assert_eq!(result2.len(), 1);

    Ok(())
}

/// step_spawner does not create duplicate tasks for the same issue.
#[tokio::test]
async fn step_spawner_no_duplicate_tasks() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/dedup")
        .build()
        .await?;
    register_agent(&harness, "swe").await;

    create_spawnable_issue(&harness, "swe", "acme/dedup", "dedup test").await?;

    // First step: should create a task.
    let first = harness.step_spawner().await?;
    assert_eq!(first.len(), 1);

    // Second step: issue already has an active task, should not create another.
    let second = harness.step_spawner().await?;
    assert!(
        second.is_empty(),
        "spawner should not create duplicate tasks for the same issue"
    );

    Ok(())
}
