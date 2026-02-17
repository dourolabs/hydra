mod harness;

use anyhow::Result;
use harness::TestHarness;
use metis_common::{
    issues::{IssueDependency, IssueDependencyType, IssueStatus, IssueType, JobSettings},
    task_status::Status,
};
use std::str::FromStr;

fn job_settings_for_repo(repo_name: &str) -> JobSettings {
    let mut settings = JobSettings::default();
    settings.repo_name =
        Some(metis_common::RepoName::from_str(repo_name).expect("valid repo name"));
    settings
}

/// Scenario 6: Multi-Repo Workflow
///
/// Tests tasks that require changes to multiple repositories. A parent issue
/// spawns two children, each targeting a different repo, with the second
/// blocked on the first. Verifies correct BundleSpec per repo and that
/// patches are created in the correct repository context.
#[tokio::test]
async fn multi_repo_workflow() -> Result<()> {
    let harness = TestHarness::builder()
        .with_repo("org/app")
        .with_repo("org/cluster")
        .with_agent("pm", "plan work")
        .with_agent("swe", "implement features")
        .build()
        .await?;

    let user = harness.default_user();
    let repo_app = metis_common::RepoName::from_str("org/app")?;
    let repo_cluster = metis_common::RepoName::from_str("org/cluster")?;

    // User creates parent issue.
    let parent_id = user.create_issue("Add new agent queue").await?;

    // PM creates child 1: targets org/app repo.
    let child1_id = user
        .create_issue_full(
            IssueType::Task,
            "Add agent queue to service.sh",
            IssueStatus::Open,
            Some("swe"),
            Some(job_settings_for_repo("org/app")),
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
            Vec::new(),
        )
        .await?;

    // PM creates child 2: targets org/cluster repo, blocked-on child 1.
    let _child2_id = user
        .create_issue_full(
            IssueType::Task,
            "Add agent queue to configmap",
            IssueStatus::Open,
            Some("swe"),
            Some(job_settings_for_repo("org/cluster")),
            vec![
                IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone()),
                IssueDependency::new(IssueDependencyType::BlockedOn, child1_id.clone()),
            ],
            Vec::new(),
        )
        .await?;

    // step_schedule: child 1 is ready, child 2 is blocked.
    let task_ids = harness.step_schedule().await?;
    assert_eq!(
        task_ids.len(),
        1,
        "only child 1 should be scheduled (child 2 is blocked)"
    );
    let job1_id = &task_ids[0];

    // SWE worker on child 1 creates patch in org/app repo.
    let result1 = harness
        .run_worker(
            job1_id,
            vec![
                "echo 'agent_queue: new-queue' >> service.sh",
                "git add service.sh",
                "git commit -m 'Add agent queue to service.sh'",
                "metis patches create --title 'Add agent queue to service.sh' --description 'Adds new agent queue config'",
            ],
        )
        .await?;

    assert_eq!(result1.final_status, Status::Complete);
    assert_eq!(result1.patches_created.len(), 1);

    // Verify patch 1 references org/app.
    let patch1 = user.get_patch(&result1.patches_created[0]).await?;
    assert_eq!(
        patch1.patch.service_repo_name, repo_app,
        "patch 1 should reference org/app"
    );

    // The patch_workflow automation may have created child issues (e.g. MergeRequest)
    // on child 1 when the patch was created. Close them before closing child 1.
    let all_issues = user.list_issues().await?;
    for issue in &all_issues.issues {
        let is_child_of_child1 =
            issue.issue.dependencies.iter().any(|d| {
                d.dependency_type == IssueDependencyType::ChildOf && d.issue_id == child1_id
            });
        if is_child_of_child1 && issue.issue.status == IssueStatus::Open {
            user.update_issue_status(&issue.issue_id, IssueStatus::Closed)
                .await?;
        }
    }

    // Close child 1 (now that its workflow children are closed).
    user.update_issue_status(&child1_id, IssueStatus::Closed)
        .await?;

    // step_schedule: child 2 is now ready (blocker resolved).
    let task_ids2 = harness.step_schedule().await?;
    assert_eq!(
        task_ids2.len(),
        1,
        "child 2 should now be scheduled (blocker resolved)"
    );
    let job2_id = &task_ids2[0];

    // SWE worker on child 2 creates patch in org/cluster repo.
    let result2 = harness
        .run_worker(
            job2_id,
            vec![
                "echo 'configmap: new-queue' >> configmap.yaml",
                "git add configmap.yaml",
                "git commit -m 'Add agent queue to configmap'",
                "metis patches create --title 'Add agent queue to configmap' --description 'Adds configmap for new agent queue'",
            ],
        )
        .await?;

    assert_eq!(result2.final_status, Status::Complete);
    assert_eq!(result2.patches_created.len(), 1);

    // Verify patch 2 references org/cluster.
    let patch2 = user.get_patch(&result2.patches_created[0]).await?;
    assert_eq!(
        patch2.patch.service_repo_name, repo_cluster,
        "patch 2 should reference org/cluster"
    );

    // Verify both patches exist and target different repos.
    assert_ne!(
        patch1.patch.service_repo_name, patch2.patch.service_repo_name,
        "patches should target different repositories"
    );

    Ok(())
}
