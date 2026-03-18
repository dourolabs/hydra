mod harness;

use anyhow::Result;
use harness::{find_summary_children_of, test_job_settings, TestHarness};
use hydra_common::{
    issues::{IssueDependencyType, IssueStatus, IssueType},
    task_status::Status,
};
use std::str::FromStr;

/// Scenario 6: Multi-Repo Workflow
///
/// Tests tasks that require changes to multiple repositories. A user creates
/// a parent issue assigned to a PM agent. The PM worker creates two child
/// issues via the CLI, each targeting a different repo, with the second
/// blocked on the first. SWE workers then pick up each child and create
/// patches in the correct repository context.
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
    let repo_app = hydra_common::RepoName::from_str("org/app")?;
    let repo_cluster = hydra_common::RepoName::from_str("org/cluster")?;

    // User creates parent issue assigned to PM.
    let parent_id = user
        .create_issue_with_settings(
            "Add new agent queue",
            IssueType::Task,
            IssueStatus::Open,
            Some("pm"),
            Some(test_job_settings(&repo_app)),
        )
        .await?;

    // PM agent spawns and creates child issues via worker CLI.
    let pm_tasks = harness.step_schedule().await?;
    assert_eq!(pm_tasks.len(), 1, "should spawn exactly one PM task");

    // PM worker creates child 1 (org/app) and child 2 (org/cluster, blocked-on child 1),
    // then sets the parent issue to in-progress.
    // We extract child 1's issue ID from the JSONL output so child 2 can reference it.
    let create_child1_cmd = format!(
        "metis --output-format jsonl issues create 'Add agent queue to service.sh' \
         --assignee swe --deps child-of:{parent_id} --repo-name org/app \
         | python3 -c \"import json,sys; print(json.load(sys.stdin)['issue_id'])\" \
         > child1_id.txt"
    );
    let create_child2_cmd = format!(
        "metis issues create 'Add agent queue to configmap' \
         --assignee swe --deps child-of:{parent_id} \
         --deps blocked-on:$(cat child1_id.txt) --repo-name org/cluster"
    );
    let set_status_cmd = format!("metis issues update {parent_id} --status in-progress");
    let pm_result = harness
        .run_worker(
            &pm_tasks[0],
            vec![&create_child1_cmd, &create_child2_cmd, &set_status_cmd],
        )
        .await?;

    assert_eq!(pm_result.final_status, Status::Complete);

    // Find the child issues created by PM.
    let all_issues = user.list_issues().await?;
    let children = find_summary_children_of(&all_issues.issues, &parent_id);
    let child1 = children
        .iter()
        .find(|i| {
            i.issue
                .description
                .contains("Add agent queue to service.sh")
        })
        .expect("PM should have created child 1");
    let child1_id = child1.issue_id.clone();

    let child2 = children
        .iter()
        .find(|i| i.issue.description.contains("Add agent queue to configmap"))
        .expect("PM should have created child 2");

    // Verify child 2 is blocked-on child 1.
    assert!(
        child2.issue.dependencies.iter().any(|d| {
            d.dependency_type == IssueDependencyType::BlockedOn && d.issue_id == child1_id
        }),
        "child 2 should be blocked-on child 1"
    );

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
    for issue in find_summary_children_of(&all_issues.issues, &child1_id) {
        if issue.issue.status == IssueStatus::Open {
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
