mod harness;

use anyhow::{Context, Result};
use harness::{IssueAssertions, PatchAssertions};
use metis::client::MetisClientInterface;
use metis_common::{
    issues::{IssueStatus, IssueType, JobSettings},
    patches::{PatchStatus, UpsertPatchRequest},
    task_status::Status,
};
use metis_server::policy::automations::patch_workflow::PatchWorkflowConfig;
use std::str::FromStr;

/// Helper to build a PatchWorkflowConfig with a reviewer and merge request.
fn test_patch_workflow_config(reviewer: &str) -> PatchWorkflowConfig {
    toml::from_str(&format!(
        r#"
[[review_requests]]
assignee = "{reviewer}"

[merge_request]
"#,
    ))
    .expect("valid PatchWorkflowConfig TOML")
}

/// Helper: set a patch status to Merged via the API, triggering
/// close_merge_request_issues automation.
async fn merge_patch(
    client: &dyn MetisClientInterface,
    patch_id: &metis_common::PatchId,
) -> Result<()> {
    let mut patch = client.get_patch(patch_id).await?;
    patch.patch.status = PatchStatus::Merged;
    let request = UpsertPatchRequest::new(patch.patch);
    client.update_patch(patch_id, &request).await?;
    Ok(())
}

/// Scenario 1: Planning agent creates sub-issues for SWE (with patch workflow)
///
/// Flow:
/// 1. User creates parent issue
/// 2. PM agent breaks it into 2 child issues with dependency ordering
///    (child 2 blocked-on child 1)
/// 3. SWE agent works child 1 (creates patch -> patch_workflow fires ->
///    review -> merge -> SWE issue re-spawns -> agent closes)
/// 4. Child 2 becomes ready -> SWE works child 2 (same cycle)
/// 5. PM closes parent via agent
///
/// Verifies:
/// - Dependency ordering is respected (child 2 waits for child 1)
/// - patch_workflow creates ReviewRequest + MergeRequest as children
/// - SWE's issue becomes spawnable after workflow children close
/// - Agent closes issues via CLI (not user)
/// - Parent closure requires all children terminal
#[tokio::test]
async fn planning_agent_creates_sub_issues_with_patch_workflow() -> Result<()> {
    let repo_str = "test-org/planning-test";
    let repo = metis_common::RepoName::from_str(repo_str)?;

    let harness = harness::TestHarness::builder()
        .with_repo(repo_str)
        .with_user("reviewer")
        .with_agent("pm", "Break down tasks for SWE agents")
        .with_agent("swe", "Implement code changes")
        .with_patch_workflow_config(test_patch_workflow_config("reviewer"))
        .build()
        .await?;

    let user = harness.default_user();
    let client = harness.client()?;

    // ── Step 1: User creates parent issue ────────────────────────────
    let mut job_settings = JobSettings::default();
    job_settings.repo_name = Some(repo.clone());
    let parent_id = user
        .create_issue_with_settings(
            "Add dark mode support",
            IssueType::Task,
            IssueStatus::Open,
            Some("pm"),
            Some(job_settings),
        )
        .await?;

    // ── Step 2: PM picks up parent issue ─────────────────────────────
    let pm_tasks = harness.step_schedule().await?;
    assert_eq!(pm_tasks.len(), 1, "spawner should create one task for PM");
    let pm_task_id = &pm_tasks[0];

    // PM worker creates child 1 and child 2 with dependency ordering,
    // and sets parent to in-progress.
    let pm_result = harness
        .run_worker(
            pm_task_id,
            vec![
                &format!(
                    "metis issues create 'Add theme toggle component' \
                     --type task --assignee swe \
                     --deps child-of:{parent_id} \
                     --repo-name {repo_str}"
                ),
                &format!("metis issues update {parent_id} --status in-progress"),
            ],
        )
        .await?;
    assert_eq!(pm_result.final_status, Status::Complete);

    // Find child 1's ID by listing issues.
    let all_issues = user.list_issues().await?;
    let child1 = all_issues
        .issues
        .iter()
        .find(|i| i.issue.description.contains("theme toggle"))
        .context("child 1 should exist")?;
    let child1_id = child1.issue_id.clone();

    // Create child 2 (blocked on child 1) via the API to set up
    // the blocked-on dependency precisely.
    let child2_id = user
        .create_issue_full(
            IssueType::Task,
            "Update CSS variables for dark theme",
            IssueStatus::Open,
            Some("swe"),
            Some({
                let mut js = JobSettings::default();
                js.repo_name = Some(repo.clone());
                js
            }),
            vec![
                metis_common::issues::IssueDependency::new(
                    metis_common::issues::IssueDependencyType::ChildOf,
                    parent_id.clone(),
                ),
                metis_common::issues::IssueDependency::new(
                    metis_common::issues::IssueDependencyType::BlockedOn,
                    child1_id.clone(),
                ),
            ],
            Vec::new(),
        )
        .await?;

    // ── Step 3: Verify structure ─────────────────────────────────────
    let parent = user.get_issue(&parent_id).await?;
    parent.assert_status(IssueStatus::InProgress);

    let all_issues = user.list_issues().await?;
    parent.assert_has_child_with_status(&all_issues.issues, "theme toggle", IssueStatus::Open);
    parent.assert_has_child_with_status(&all_issues.issues, "CSS variables", IssueStatus::Open);

    // Verify child 2 has blocked-on child 1.
    let child2 = user.get_issue(&child2_id).await?;
    assert!(
        child2.issue.dependencies.iter().any(|d| {
            d.dependency_type == metis_common::issues::IssueDependencyType::BlockedOn
                && d.issue_id == child1_id
        }),
        "child 2 should be blocked-on child 1"
    );

    // ── Step 4: Schedule — child 1 ready, child 2 blocked ───────────
    let swe_tasks = harness.step_schedule().await?;
    assert_eq!(
        swe_tasks.len(),
        1,
        "only child 1 should be spawned (child 2 is blocked)"
    );
    let swe1_task_id = &swe_tasks[0];

    // ── Step 5: SWE worker executes on child 1 ──────────────────────
    let swe1_result = harness
        .run_worker(
            swe1_task_id,
            vec![
                "echo 'toggle component code' >> README.md",
                "git add README.md",
                "git commit -m 'Add theme toggle component'",
                "metis patches create --title 'Add theme toggle' --description 'Implements toggle'",
            ],
        )
        .await?;
    assert_eq!(swe1_result.final_status, Status::Complete);
    assert_eq!(
        swe1_result.patches_created.len(),
        1,
        "SWE should create one patch"
    );
    let patch1_id = &swe1_result.patches_created[0];

    // ── Step 6: Verify patch_workflow automation fired ────────────────
    // The patch_workflow should have created ReviewRequest + MergeRequest
    // as children of child 1.
    let all_issues = user.list_issues().await?;

    // Find the ReviewRequest and MergeRequest issues.
    let child1_children: Vec<_> = all_issues
        .issues
        .iter()
        .filter(|i| {
            i.issue.dependencies.iter().any(|d| {
                d.dependency_type == metis_common::issues::IssueDependencyType::ChildOf
                    && d.issue_id == child1_id
            })
        })
        .collect();

    assert!(
        child1_children.len() >= 2,
        "child 1 should have at least 2 children (ReviewRequest + MergeRequest), got {}",
        child1_children.len()
    );

    let review_request1 = child1_children
        .iter()
        .find(|i| i.issue.issue_type == IssueType::ReviewRequest)
        .context("ReviewRequest should exist as child of child 1")?;
    let merge_request1 = child1_children
        .iter()
        .find(|i| i.issue.issue_type == IssueType::MergeRequest)
        .context("MergeRequest should exist as child of child 1")?;

    assert_eq!(
        review_request1.issue.assignee.as_deref(),
        Some("reviewer"),
        "ReviewRequest should be assigned to reviewer"
    );

    // MergeRequest should be blocked on ReviewRequest.
    assert!(
        merge_request1.issue.dependencies.iter().any(|d| {
            d.dependency_type == metis_common::issues::IssueDependencyType::BlockedOn
                && d.issue_id == review_request1.issue_id
        }),
        "MergeRequest should be blocked on ReviewRequest"
    );

    // ── Step 7: Reviewer approves the patch ──────────────────────────
    harness
        .user("reviewer")
        .cli(&[
            "patches",
            "review",
            patch1_id.as_ref(),
            "--author",
            "reviewer",
            "--contents",
            "looks good",
            "--approve",
        ])
        .await?;

    // sync_review_request_issues should close the ReviewRequest.
    harness.step_github_sync().await?;

    let review_request1_updated = user.get_issue(&review_request1.issue_id).await?;
    review_request1_updated.assert_status(IssueStatus::Closed);

    // ── Step 8: Merge the patch ──────────────────────────────────────
    // Update patch status to Merged via API, triggering
    // close_merge_request_issues to close the MergeRequest.
    merge_patch(&client, patch1_id).await?;

    let patch1 = user.get_patch(patch1_id).await?;
    patch1.assert_status(PatchStatus::Merged);

    let merge_request1_updated = user.get_issue(&merge_request1.issue_id).await?;
    merge_request1_updated.assert_status(IssueStatus::Closed);

    // ── Step 9: SWE issue re-spawns and agent closes child 1 ────────
    // All workflow children are terminal, so child 1 (still Open/InProgress)
    // becomes spawnable again. The agent closes it via the CLI.
    let swe1_close_tasks = harness.step_schedule().await?;
    assert_eq!(
        swe1_close_tasks.len(),
        1,
        "child 1 should be re-spawned after workflow children close"
    );

    harness
        .run_worker(
            &swe1_close_tasks[0],
            vec![&format!("metis issues update {child1_id} --status closed")],
        )
        .await?;

    let child1_closed = user.get_issue(&child1_id).await?;
    child1_closed.assert_status(IssueStatus::Closed);

    // ── Step 10: Child 2 should now be ready ─────────────────────────
    let swe2_tasks = harness.step_schedule().await?;
    assert_eq!(
        swe2_tasks.len(),
        1,
        "child 2 should now be spawned (blocker resolved)"
    );
    let swe2_task_id = &swe2_tasks[0];

    // ── Step 11: SWE worker executes on child 2 ─────────────────────
    let swe2_result = harness
        .run_worker(
            swe2_task_id,
            vec![
                "echo 'css variables code' >> README.md",
                "git add README.md",
                "git commit -m 'Update CSS variables for dark theme'",
                "metis patches create --title 'Update CSS variables' --description 'Dark theme CSS'",
            ],
        )
        .await?;
    assert_eq!(swe2_result.final_status, Status::Complete);
    assert_eq!(swe2_result.patches_created.len(), 1);
    let patch2_id = &swe2_result.patches_created[0];

    // Verify patch_workflow fired on child 2 as well.
    let all_issues = user.list_issues().await?;
    let child2_children: Vec<_> = all_issues
        .issues
        .iter()
        .filter(|i| {
            i.issue.dependencies.iter().any(|d| {
                d.dependency_type == metis_common::issues::IssueDependencyType::ChildOf
                    && d.issue_id == child2_id
            })
        })
        .collect();

    let review_request2 = child2_children
        .iter()
        .find(|i| i.issue.issue_type == IssueType::ReviewRequest)
        .context("ReviewRequest should exist as child of child 2")?;
    let merge_request2 = child2_children
        .iter()
        .find(|i| i.issue.issue_type == IssueType::MergeRequest)
        .context("MergeRequest should exist as child of child 2")?;

    // ── Step 12: Review and merge child 2's patch ────────────────────
    harness
        .user("reviewer")
        .cli(&[
            "patches",
            "review",
            patch2_id.as_ref(),
            "--author",
            "reviewer",
            "--contents",
            "approved",
            "--approve",
        ])
        .await?;

    harness.step_github_sync().await?;

    let rr2 = user.get_issue(&review_request2.issue_id).await?;
    rr2.assert_status(IssueStatus::Closed);

    merge_patch(&client, patch2_id).await?;

    let patch2 = user.get_patch(patch2_id).await?;
    patch2.assert_status(PatchStatus::Merged);

    let mr2 = user.get_issue(&merge_request2.issue_id).await?;
    mr2.assert_status(IssueStatus::Closed);

    // ── Step 13: SWE issue re-spawns and agent closes child 2 ───────
    let swe2_close_tasks = harness.step_schedule().await?;
    assert_eq!(
        swe2_close_tasks.len(),
        1,
        "child 2 should be re-spawned after workflow children close"
    );

    harness
        .run_worker(
            &swe2_close_tasks[0],
            vec![&format!("metis issues update {child2_id} --status closed")],
        )
        .await?;

    let child2_closed = user.get_issue(&child2_id).await?;
    child2_closed.assert_status(IssueStatus::Closed);

    // ── Step 14: PM re-spawns and closes parent ─────────────────────
    // All children are terminal, so parent becomes spawnable again.
    let pm_close_tasks = harness.step_schedule().await?;
    assert_eq!(
        pm_close_tasks.len(),
        1,
        "parent should be re-spawned after all children close"
    );

    harness
        .run_worker(
            &pm_close_tasks[0],
            vec![&format!("metis issues update {parent_id} --status closed")],
        )
        .await?;

    let parent_closed = user.get_issue(&parent_id).await?;
    parent_closed.assert_status(IssueStatus::Closed);

    Ok(())
}

/// Scenario 5: SWE agent failure and re-planning
///
/// Flow:
/// 1. User creates parent issue
/// 2. PM creates two children: child 1 assigned to SWE, child 2 blocked-on child 1
/// 3. SWE agent explicitly fails child 1 via CLI
/// 4. Child 2 is automatically dropped (blocked on failed task)
/// 5. Parent becomes ready for re-spawning (in-progress, no ready descendants)
/// 6. PM creates replacement child with updated instructions
/// 7. SWE succeeds on replacement child and closes it
/// 8. PM closes parent
///
/// Verifies:
/// - Agent explicitly sets issue status to failed via CLI
/// - Failed child does not block parent re-planning
/// - Blocked sibling does not prevent PM from re-spawning
/// - Parent in-progress with no ready descendants becomes ready
/// - Replacement child issues work correctly
/// - Original failed child remains in terminal state
#[tokio::test]
async fn swe_agent_failure_triggers_replanning() -> Result<()> {
    let repo_str = "test-org/replan-test";

    let harness = harness::TestHarness::builder()
        .with_repo(repo_str)
        .with_agent("pm", "Plan and coordinate tasks")
        .with_agent("swe", "Implement code changes")
        .build()
        .await?;

    let user = harness.default_user();

    // ── Step 1: User creates parent issue ────────────────────────────
    let mut job_settings = JobSettings::default();
    job_settings.repo_name = Some(metis_common::RepoName::from_str(repo_str)?);
    let parent_id = user
        .create_issue_with_settings(
            "Implement caching layer",
            IssueType::Task,
            IssueStatus::Open,
            Some("pm"),
            Some(job_settings.clone()),
        )
        .await?;

    // ── Step 2: PM picks up parent and creates two children ──────────
    let pm_tasks = harness.step_schedule().await?;
    assert_eq!(pm_tasks.len(), 1);

    // PM worker creates child 1 and sets parent to in-progress.
    harness
        .run_worker(
            &pm_tasks[0],
            vec![
                &format!(
                    "metis issues create 'Add Redis cache integration' \
                     --type task --assignee swe \
                     --deps child-of:{parent_id} \
                     --repo-name {repo_str}"
                ),
                &format!("metis issues update {parent_id} --status in-progress"),
            ],
        )
        .await?;

    // Find child 1's ID.
    let all_issues = user.list_issues().await?;
    let child1 = all_issues
        .issues
        .iter()
        .find(|i| i.issue.description.contains("Redis cache"))
        .context("child 1 should exist")?;
    let child1_id = child1.issue_id.clone();

    // Create child 2 blocked-on child 1 to verify it doesn't prevent re-planning.
    let child2_id = user
        .create_issue_full(
            IssueType::Task,
            "Add cache invalidation logic",
            IssueStatus::Open,
            Some("swe"),
            Some(job_settings),
            vec![
                metis_common::issues::IssueDependency::new(
                    metis_common::issues::IssueDependencyType::ChildOf,
                    parent_id.clone(),
                ),
                metis_common::issues::IssueDependency::new(
                    metis_common::issues::IssueDependencyType::BlockedOn,
                    child1_id.clone(),
                ),
            ],
            Vec::new(),
        )
        .await?;

    // Verify parent is in-progress.
    let parent = user.get_issue(&parent_id).await?;
    parent.assert_status(IssueStatus::InProgress);

    // ── Step 3: SWE picks up child 1 and explicitly fails ────────────
    let swe_tasks = harness.step_schedule().await?;
    assert_eq!(swe_tasks.len(), 1, "child 1 should be spawned for SWE");
    let swe_task_id = &swe_tasks[0];

    // The agent decides the task is impossible and sets its status to Failed via CLI.
    let swe_result = harness
        .run_worker(
            swe_task_id,
            vec![&format!("metis issues update {child1_id} --status failed")],
        )
        .await?;
    assert_eq!(swe_result.final_status, Status::Complete);

    let child1_failed = user.get_issue(&child1_id).await?;
    child1_failed.assert_status(IssueStatus::Failed);

    // ── Step 4: Verify child 2 state ─────────────────────────────────
    // Child 2 is blocked-on the failed child 1. It should not be ready.
    let child2_check = user.get_issue(&child2_id).await?;
    assert!(
        child2_check.issue.status == IssueStatus::Open
            || child2_check.issue.status == IssueStatus::Dropped,
        "child 2 should be open or dropped (blocked on failed child 1), got {:?}",
        child2_check.issue.status
    );

    // ── Step 5: Parent becomes ready for re-spawning ─────────────────
    // Parent is in-progress with no ready descendants (child 1 is failed,
    // child 2 is blocked/dropped). The spawner should create a new task
    // for the parent.
    let pm_tasks_round2 = harness.step_schedule().await?;
    assert_eq!(
        pm_tasks_round2.len(),
        1,
        "parent should be re-spawned (no ready descendants)"
    );

    // ── Step 6: PM drops blocked child 2 and creates replacement ──────
    // The PM drops child 2 (blocked on the failed task) and creates a
    // replacement child with updated instructions.
    harness
        .run_worker(
            &pm_tasks_round2[0],
            vec![
                &format!("metis issues update {child2_id} --status dropped"),
                &format!(
                    "metis issues create 'Add Memcached cache integration (retry)' \
                     --type task --assignee swe \
                     --deps child-of:{parent_id} \
                     --repo-name {repo_str}"
                ),
            ],
        )
        .await?;

    // Verify child 2 is dropped.
    let child2_dropped = user.get_issue(&child2_id).await?;
    child2_dropped.assert_status(IssueStatus::Dropped);

    // Find the new child issue.
    let all_issues = user.list_issues().await?;
    let child3 = all_issues
        .issues
        .iter()
        .find(|i| i.issue.description.contains("Memcached"))
        .context("replacement child should exist")?;
    let child3_id = child3.issue_id.clone();

    // Verify original child is still failed.
    let child1_still_failed = user.get_issue(&child1_id).await?;
    child1_still_failed.assert_status(IssueStatus::Failed);

    // Verify new child is open.
    let child3_check = user.get_issue(&child3_id).await?;
    child3_check.assert_status(IssueStatus::Open);

    // ── Step 7: SWE succeeds on replacement child and closes it ──────
    let swe_tasks_round2 = harness.step_schedule().await?;
    assert_eq!(
        swe_tasks_round2.len(),
        1,
        "replacement child should be spawned"
    );

    let result = harness
        .run_worker(
            &swe_tasks_round2[0],
            vec![
                "echo 'cache implementation' >> README.md",
                "git add README.md",
                "git commit -m 'Add Memcached cache integration'",
                &format!("metis issues update {child3_id} --status closed"),
            ],
        )
        .await?;
    assert_eq!(result.final_status, Status::Complete);

    let child3_closed = user.get_issue(&child3_id).await?;
    child3_closed.assert_status(IssueStatus::Closed);

    // ── Step 8: PM re-spawns and closes parent ──────────────────────
    // All children are terminal (child 1 failed, child 2 blocked/dropped,
    // child 3 closed), so parent becomes spawnable again.
    let pm_close_tasks = harness.step_schedule().await?;
    assert_eq!(
        pm_close_tasks.len(),
        1,
        "parent should be re-spawned after all descendants are terminal"
    );

    harness
        .run_worker(
            &pm_close_tasks[0],
            vec![&format!("metis issues update {parent_id} --status closed")],
        )
        .await?;

    // ── Final verification ───────────────────────────────────────────
    let parent_final = user.get_issue(&parent_id).await?;
    parent_final.assert_status(IssueStatus::Closed);

    let child1_final = user.get_issue(&child1_id).await?;
    child1_final.assert_status(IssueStatus::Failed);

    let child3_final = user.get_issue(&child3_id).await?;
    child3_final.assert_status(IssueStatus::Closed);

    // Verify the parent has the correct children structure.
    let all_issues = user.list_issues().await?;
    parent_final.assert_has_child_with_status(
        &all_issues.issues,
        "Redis cache",
        IssueStatus::Failed,
    );
    parent_final.assert_has_child_with_status(&all_issues.issues, "Memcached", IssueStatus::Closed);

    Ok(())
}

/// Scenario 5b: User rejects plan and triggers re-planning
///
/// Flow:
/// 1. User creates parent issue
/// 2. PM creates child assigned to SWE, sets parent to in-progress
/// 3. SWE picks up the child (job starts running)
/// 4. User rejects the child issue (sets status to rejected)
/// 5. Parent becomes ready for re-spawning
/// 6. PM creates replacement child
/// 7. SWE succeeds on replacement child and closes it
/// 8. PM closes parent
///
/// Verifies:
/// - User can reject an issue to trigger re-planning
/// - Rejected issue is terminal and does not block parent
/// - PM re-spawns after rejection
/// - Replacement child completes the work
#[tokio::test]
async fn user_rejects_plan_triggers_replanning() -> Result<()> {
    let repo_str = "test-org/reject-test";

    let harness = harness::TestHarness::builder()
        .with_repo(repo_str)
        .with_agent("pm", "Plan and coordinate tasks")
        .with_agent("swe", "Implement code changes")
        .build()
        .await?;

    let user = harness.default_user();

    // ── Step 1: User creates parent issue ────────────────────────────
    let mut job_settings = JobSettings::default();
    job_settings.repo_name = Some(metis_common::RepoName::from_str(repo_str)?);
    let parent_id = user
        .create_issue_with_settings(
            "Implement search feature",
            IssueType::Task,
            IssueStatus::Open,
            Some("pm"),
            Some(job_settings.clone()),
        )
        .await?;

    // ── Step 2: PM picks up parent and creates child ─────────────────
    let pm_tasks = harness.step_schedule().await?;
    assert_eq!(pm_tasks.len(), 1);

    harness
        .run_worker(
            &pm_tasks[0],
            vec![
                &format!(
                    "metis issues create 'Build full-text search with Elasticsearch' \
                     --type task --assignee swe \
                     --deps child-of:{parent_id} \
                     --repo-name {repo_str}"
                ),
                &format!("metis issues update {parent_id} --status in-progress"),
            ],
        )
        .await?;

    // Find the child issue.
    let all_issues = user.list_issues().await?;
    let child1 = all_issues
        .issues
        .iter()
        .find(|i| i.issue.description.contains("Elasticsearch"))
        .context("child 1 should exist")?;
    let child1_id = child1.issue_id.clone();

    // Verify parent is in-progress.
    let parent = user.get_issue(&parent_id).await?;
    parent.assert_status(IssueStatus::InProgress);

    // ── Step 3: SWE picks up child (job starts) ──────────────────────
    let swe_tasks = harness.step_schedule().await?;
    assert_eq!(swe_tasks.len(), 1, "child should be spawned for SWE");

    // ── Step 4: User rejects the child issue ─────────────────────────
    // User decides they don't like the plan and sets the issue to rejected.
    user.update_issue_status(&child1_id, IssueStatus::Rejected)
        .await?;

    let child1_rejected = user.get_issue(&child1_id).await?;
    child1_rejected.assert_status(IssueStatus::Rejected);

    // Reconcile task status: the kill_tasks_on_issue_failure automation
    // killed the SWE job in the engine, but the task record still shows
    // Running. step_monitor_jobs reconciles the store with the engine.
    harness.step_monitor_jobs().await?;

    // ── Step 5: Parent becomes ready for re-spawning ─────────────────
    // Parent is in-progress with no ready descendants (child is rejected).
    let pm_tasks_round2 = harness.step_schedule().await?;
    assert_eq!(
        pm_tasks_round2.len(),
        1,
        "parent should be re-spawned after child is rejected"
    );

    // ── Step 6: PM creates replacement child ─────────────────────────
    harness
        .run_worker(
            &pm_tasks_round2[0],
            vec![&format!(
                "metis issues create 'Build search with SQLite FTS5' \
                 --type task --assignee swe \
                 --deps child-of:{parent_id} \
                 --repo-name {repo_str}"
            )],
        )
        .await?;

    // Find the new child issue.
    let all_issues = user.list_issues().await?;
    let child2 = all_issues
        .issues
        .iter()
        .find(|i| i.issue.description.contains("SQLite FTS5"))
        .context("replacement child should exist")?;
    let child2_id = child2.issue_id.clone();

    // Verify original child is still rejected.
    let child1_still_rejected = user.get_issue(&child1_id).await?;
    child1_still_rejected.assert_status(IssueStatus::Rejected);

    // Verify new child is open.
    let child2_check = user.get_issue(&child2_id).await?;
    child2_check.assert_status(IssueStatus::Open);

    // ── Step 7: SWE succeeds on replacement child and closes it ──────
    let swe_tasks_round2 = harness.step_schedule().await?;
    assert_eq!(
        swe_tasks_round2.len(),
        1,
        "replacement child should be spawned"
    );

    harness
        .run_worker(
            &swe_tasks_round2[0],
            vec![
                "echo 'search implementation' >> README.md",
                "git add README.md",
                "git commit -m 'Build search with SQLite FTS5'",
                &format!("metis issues update {child2_id} --status closed"),
            ],
        )
        .await?;

    let child2_closed = user.get_issue(&child2_id).await?;
    child2_closed.assert_status(IssueStatus::Closed);

    // ── Step 8: PM re-spawns and closes parent ──────────────────────
    let pm_close_tasks = harness.step_schedule().await?;
    assert_eq!(
        pm_close_tasks.len(),
        1,
        "parent should be re-spawned after all children are terminal"
    );

    harness
        .run_worker(
            &pm_close_tasks[0],
            vec![&format!("metis issues update {parent_id} --status closed")],
        )
        .await?;

    // ── Final verification ───────────────────────────────────────────
    let parent_final = user.get_issue(&parent_id).await?;
    parent_final.assert_status(IssueStatus::Closed);

    let child1_final = user.get_issue(&child1_id).await?;
    child1_final.assert_status(IssueStatus::Rejected);

    let child2_final = user.get_issue(&child2_id).await?;
    child2_final.assert_status(IssueStatus::Closed);

    // Verify children structure.
    let all_issues = user.list_issues().await?;
    parent_final.assert_has_child_with_status(
        &all_issues.issues,
        "Elasticsearch",
        IssueStatus::Rejected,
    );
    parent_final.assert_has_child_with_status(
        &all_issues.issues,
        "SQLite FTS5",
        IssueStatus::Closed,
    );

    Ok(())
}
