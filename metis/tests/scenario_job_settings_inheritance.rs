mod harness;

use anyhow::Result;
use harness::{JobAssertions, TestHarness};
use metis_common::{
    issues::{IssueDependencyType, IssueStatus, IssueType, JobSettings},
    jobs::BundleSpec,
};
use std::str::FromStr;

/// Scenario 16: Job Settings Inheritance
///
/// Tests that job settings (repo_name, image, model, cpu_limit, memory_limit)
/// configured on an issue are correctly inherited by spawned tasks. Verifies
/// that the spawned task has the correct BundleSpec, resource limits, and
/// METIS_ISSUE_ID env var.
#[tokio::test]
async fn job_settings_inheritance_through_spawning_pipeline() -> Result<()> {
    let harness = TestHarness::builder()
        .with_repo("acme/settings-test")
        .with_agent("swe", "implement features")
        .build()
        .await?;

    let user = harness.default_user();
    let repo = metis_common::RepoName::from_str("acme/settings-test")?;

    // Create an issue with full job settings.
    let mut job_settings = JobSettings::default();
    job_settings.repo_name = Some(repo.clone());
    job_settings.image = Some("custom-worker:v2".to_string());
    job_settings.model = Some("claude-opus-4-20250514".to_string());
    job_settings.cpu_limit = Some("4".to_string());
    job_settings.memory_limit = Some("8Gi".to_string());

    let issue_id = user
        .create_issue_with_settings(
            "Task with custom job settings",
            IssueType::Task,
            IssueStatus::Open,
            Some("swe"),
            Some(job_settings),
        )
        .await?;

    // step_schedule() spawns a task for the issue.
    let task_ids = harness.step_schedule().await?;
    assert_eq!(task_ids.len(), 1, "should spawn exactly one task");
    let job_id = &task_ids[0];

    // Retrieve the spawned task and verify inherited settings.
    let job = user.client().get_job(job_id).await?;

    // Verify image is inherited.
    assert_eq!(
        job.task.image.as_deref(),
        Some("custom-worker:v2"),
        "spawned task should inherit the image from job settings"
    );

    // Verify model is inherited.
    assert_eq!(
        job.task.model.as_deref(),
        Some("claude-opus-4-20250514"),
        "spawned task should inherit the model from job settings"
    );

    // Verify cpu_limit is inherited.
    assert_eq!(
        job.task.cpu_limit.as_deref(),
        Some("4"),
        "spawned task should inherit the cpu_limit from job settings"
    );

    // Verify memory_limit is inherited.
    assert_eq!(
        job.task.memory_limit.as_deref(),
        Some("8Gi"),
        "spawned task should inherit the memory_limit from job settings"
    );

    // Verify BundleSpec references the correct repository.
    match &job.task.context {
        BundleSpec::ServiceRepository { name, .. } => {
            assert_eq!(
                name, &repo,
                "BundleSpec should reference the correct repository"
            );
        }
        other => {
            panic!("expected BundleSpec::ServiceRepository, got {other:?}");
        }
    }

    // Verify METIS_ISSUE_ID env var is set.
    job.assert_env_var("METIS_ISSUE_ID", issue_id.as_ref());

    Ok(())
}

/// Verify that a PM worker creating a child issue via CLI with --repo-name
/// results in the child's spawned task having the correct BundleSpec and env vars.
#[tokio::test]
async fn pm_creates_child_with_repo_settings_via_cli() -> Result<()> {
    let harness = TestHarness::builder()
        .with_repo("acme/child-test")
        .with_agent("pm", "plan work")
        .with_agent("swe", "implement features")
        .build()
        .await?;

    let user = harness.default_user();
    let repo = metis_common::RepoName::from_str("acme/child-test")?;

    // Create parent issue with repo job settings (assigned to PM).
    let mut parent_settings = JobSettings::default();
    parent_settings.repo_name = Some(repo.clone());

    let parent_id = user
        .create_issue_with_settings(
            "Parent issue for child test",
            IssueType::Task,
            IssueStatus::Open,
            Some("pm"),
            Some(parent_settings),
        )
        .await?;

    // PM agent spawns and creates a child issue via worker CLI.
    let pm_tasks = harness.step_schedule().await?;
    assert_eq!(pm_tasks.len(), 1);

    let create_cmd = format!(
        "metis issues create 'Implement child feature' --assignee swe --deps child-of:{parent_id} --repo-name acme/child-test"
    );
    let set_status_cmd = format!("metis issues update {parent_id} --status in-progress");
    let _pm_result = harness
        .run_worker(&pm_tasks[0], vec![&create_cmd, &set_status_cmd])
        .await?;

    // Find the child issue created by PM.
    let all_issues = user.list_issues().await?;
    let child = all_issues
        .issues
        .iter()
        .find(|i| {
            i.issue.description.contains("Implement child feature")
                && i.issue.dependencies.iter().any(|d| {
                    d.dependency_type == IssueDependencyType::ChildOf && d.issue_id == parent_id
                })
        })
        .expect("PM should have created a child issue");

    // Verify the child issue has repo job settings.
    assert_eq!(
        child.issue.job_settings.repo_name,
        Some(repo.clone()),
        "child issue should have repo_name set"
    );

    // Schedule the child issue (SWE picks it up).
    let swe_tasks = harness.step_schedule().await?;
    assert_eq!(swe_tasks.len(), 1, "child should be scheduled for SWE");

    // Verify the spawned task has the correct BundleSpec.
    let child_job = user.client().get_job(&swe_tasks[0]).await?;
    match &child_job.task.context {
        BundleSpec::ServiceRepository { name, .. } => {
            assert_eq!(
                name, &repo,
                "child task BundleSpec should reference the correct repository"
            );
        }
        other => {
            panic!("expected BundleSpec::ServiceRepository, got {other:?}");
        }
    }

    // Verify METIS_ISSUE_ID is set to the child issue ID.
    child_job.assert_env_var("METIS_ISSUE_ID", child.issue_id.as_ref());

    Ok(())
}
