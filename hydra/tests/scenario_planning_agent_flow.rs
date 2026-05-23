mod harness;

use anyhow::{Context, Result};
use harness::{find_issue_summary_by_description, test_job_settings, IssueAssertions};
use hydra_common::{
    issues::{IssueStatus, IssueType},
    task_status::Status,
};
use std::str::FromStr;

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
    let repo = hydra_common::RepoName::from_str(repo_str)?;
    let parent_id = user
        .create_issue_with_settings(
            "Implement caching layer",
            IssueType::Task,
            IssueStatus::Open,
            Some("pm"),
            Some(test_job_settings(&repo)),
        )
        .await?;

    // ── Step 2: PM picks up parent and creates two children ──────────
    let pm_tasks = harness.await_sessions(&parent_id, 1).await?;
    assert_eq!(pm_tasks.len(), 1);

    // PM worker creates child 1 and sets parent to in-progress.
    harness
        .run_worker(
            &pm_tasks[0],
            vec![
                &format!(
                    "hydra issues create 'Add Redis cache integration' \
                     --type task --assignee swe \
                     --deps child-of:{parent_id} \
                     --repo-name {repo_str}"
                ),
                &format!("hydra issues update {parent_id} --status in-progress"),
            ],
        )
        .await?;

    // Find child 1's ID.
    let all_issues = user.list_issues().await?;
    let child1 = find_issue_summary_by_description(&all_issues.issues, "Redis cache")
        .context("child 1 should exist")?;
    let child1_id = child1.issue_id.clone();

    // Create child 2 blocked-on child 1 to verify it doesn't prevent re-planning.
    let child2_id = user
        .create_issue_full(
            IssueType::Task,
            "Add cache invalidation logic",
            IssueStatus::Open,
            Some("swe"),
            Some(test_job_settings(&repo)),
            vec![
                hydra_common::issues::IssueDependency::new(
                    hydra_common::issues::IssueDependencyType::ChildOf,
                    parent_id.clone(),
                ),
                hydra_common::issues::IssueDependency::new(
                    hydra_common::issues::IssueDependencyType::BlockedOn,
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
    let swe_tasks = harness.await_sessions(&child1_id, 1).await?;
    assert_eq!(swe_tasks.len(), 1, "child 1 should be spawned for SWE");
    let swe_task_id = &swe_tasks[0];

    // The agent decides the task is impossible and sets its status to Failed via CLI.
    let swe_result = harness
        .run_worker(
            swe_task_id,
            vec![&format!("hydra issues update {child1_id} --status failed")],
        )
        .await?;
    assert_eq!(swe_result.final_status, Status::Complete);

    let child1_failed = user.get_issue(&child1_id).await?;
    child1_failed.assert_status(IssueStatus::Failed);

    // ── Step 4: Verify child 2 state ─────────────────────────────────
    // Child 2 is blocked-on the failed child 1. It should remain open (not dropped).
    let child2_check = user.get_issue(&child2_id).await?;
    child2_check.assert_status(IssueStatus::Open);

    // ── Step 5: Parent becomes ready for re-spawning ─────────────────
    // Parent is in-progress with no ready descendants (child 1 is failed,
    // child 2 is blocked/dropped). The spawner should create a new task
    // for the parent.
    let pm_tasks_round2 = harness.await_sessions(&parent_id, 2).await?;
    assert_eq!(
        pm_tasks_round2.len(),
        2,
        "parent should have two sessions (original + re-spawn)"
    );
    let pm_task_round2 = pm_tasks_round2
        .iter()
        .find(|id| !pm_tasks.contains(id))
        .expect("should find a new session for parent re-spawn");

    // ── Step 6: PM drops blocked child 2 and creates replacement ──────
    // The PM drops child 2 (blocked on the failed task) and creates a
    // replacement child with updated instructions.
    harness
        .run_worker(
            pm_task_round2,
            vec![
                &format!("hydra issues update {child2_id} --status dropped"),
                &format!(
                    "hydra issues create 'Add Memcached cache integration (retry)' \
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
    let child3 = find_issue_summary_by_description(&all_issues.issues, "Memcached")
        .context("replacement child should exist")?;
    let child3_id = child3.issue_id.clone();

    // Verify original child is still failed.
    let child1_still_failed = user.get_issue(&child1_id).await?;
    child1_still_failed.assert_status(IssueStatus::Failed);

    // Verify new child is open.
    let child3_check = user.get_issue(&child3_id).await?;
    child3_check.assert_status(IssueStatus::Open);

    // ── Step 7: SWE succeeds on replacement child and closes it ──────
    let swe_tasks_round2 = harness.await_sessions(&child3_id, 1).await?;
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
                &format!("hydra issues update {child3_id} --status closed"),
            ],
        )
        .await?;
    assert_eq!(result.final_status, Status::Complete);

    let child3_closed = user.get_issue(&child3_id).await?;
    child3_closed.assert_status(IssueStatus::Closed);

    // ── Step 8: PM re-spawns and closes parent ──────────────────────
    // All children are terminal (child 1 failed, child 2 blocked/dropped,
    // child 3 closed), so parent becomes spawnable again.
    let pm_close_tasks = harness.await_sessions(&parent_id, 3).await?;
    assert_eq!(
        pm_close_tasks.len(),
        3,
        "parent should have three sessions after second re-spawn"
    );
    let pm_close_task = pm_close_tasks
        .iter()
        .find(|id| !pm_tasks_round2.contains(id))
        .expect("should find a new session for parent second re-spawn");

    harness
        .run_worker(
            pm_close_task,
            vec![&format!("hydra issues update {parent_id} --status closed")],
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
    parent_final.assert_has_child_with_status_in_summaries(
        &all_issues.issues,
        "Redis cache",
        IssueStatus::Failed,
    );
    parent_final.assert_has_child_with_status_in_summaries(
        &all_issues.issues,
        "Memcached",
        IssueStatus::Closed,
    );

    Ok(())
}

/// Scenario 5b: User rejects plan and triggers re-planning
///
/// Flow:
/// 1. User creates parent issue
/// 2. PM creates two children: child 1 assigned to SWE, child 2 blocked-on child 1
/// 3. SWE picks up child 1 (job starts running)
/// 4. User drops child 1 (sets status to dropped)
/// 5. Child 2 remains open but blocked (not ready)
/// 6. Parent becomes ready for re-spawning (no ready descendants)
/// 7. PM drops child 2 and creates replacement child
/// 8. SWE succeeds on replacement child and closes it
/// 9. PM closes parent
///
/// Verifies:
/// - User can drop an issue to trigger re-planning
/// - Dropped issue is terminal and does not block parent
/// - Blocked sibling (not explicitly dropped) does not prevent PM from re-spawning
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
    let repo = hydra_common::RepoName::from_str(repo_str)?;
    let parent_id = user
        .create_issue_with_settings(
            "Implement search feature",
            IssueType::Task,
            IssueStatus::Open,
            Some("pm"),
            Some(test_job_settings(&repo)),
        )
        .await?;

    // ── Step 2: PM picks up parent and creates two children ──────────
    let pm_tasks = harness.await_sessions(&parent_id, 1).await?;
    assert_eq!(pm_tasks.len(), 1);

    harness
        .run_worker(
            &pm_tasks[0],
            vec![
                &format!(
                    "hydra issues create 'Build full-text search with Elasticsearch' \
                     --type task --assignee swe \
                     --deps child-of:{parent_id} \
                     --repo-name {repo_str}"
                ),
                &format!("hydra issues update {parent_id} --status in-progress"),
            ],
        )
        .await?;

    // Find child 1's ID.
    let all_issues = user.list_issues().await?;
    let child1 = find_issue_summary_by_description(&all_issues.issues, "Elasticsearch")
        .context("child 1 should exist")?;
    let child1_id = child1.issue_id.clone();

    // Create child 2 blocked-on child 1 to verify it doesn't prevent re-planning.
    let child2_id = user
        .create_issue_full(
            IssueType::Task,
            "Add search result ranking",
            IssueStatus::Open,
            Some("swe"),
            Some(test_job_settings(&repo)),
            vec![
                hydra_common::issues::IssueDependency::new(
                    hydra_common::issues::IssueDependencyType::ChildOf,
                    parent_id.clone(),
                ),
                hydra_common::issues::IssueDependency::new(
                    hydra_common::issues::IssueDependencyType::BlockedOn,
                    child1_id.clone(),
                ),
            ],
            Vec::new(),
        )
        .await?;

    // Verify parent is in-progress.
    let parent = user.get_issue(&parent_id).await?;
    parent.assert_status(IssueStatus::InProgress);

    // ── Step 3: SWE picks up child 1 (job starts) ───────────────────
    let swe_tasks = harness.await_sessions(&child1_id, 1).await?;
    assert_eq!(
        swe_tasks.len(),
        1,
        "child 1 should have exactly one session"
    );

    // ── Step 4: User drops child 1 ────────────────────────────────
    // User decides they don't like the plan and sets child 1 to dropped.
    user.update_issue_status(&child1_id, IssueStatus::Dropped)
        .await?;

    let child1_dropped = user.get_issue(&child1_id).await?;
    child1_dropped.assert_status(IssueStatus::Dropped);

    // Reconcile task status: the kill_tasks_on_issue_failure automation
    // killed the SWE job in the engine, but the task record still shows
    // Running. step_monitor_jobs reconciles the store with the engine.
    harness.step_monitor_jobs().await?;

    // ── Step 5: Verify child 2 state ────────────────────────────────
    // Child 2 is blocked-on the dropped child 1. It should remain open.
    let child2_check = user.get_issue(&child2_id).await?;
    child2_check.assert_status(IssueStatus::Open);

    // ── Step 6: Parent becomes ready for re-spawning ─────────────────
    // Parent is in-progress with no ready descendants (child 1 is dropped,
    // child 2 is blocked). The spawner should create a new task for the parent.
    let pm_tasks_round2 = harness.await_sessions(&parent_id, 2).await?;
    assert_eq!(
        pm_tasks_round2.len(),
        2,
        "parent should have two sessions (original + re-spawn after rejection)"
    );
    let pm_task_round2 = pm_tasks_round2
        .iter()
        .find(|id| !pm_tasks.contains(id))
        .expect("should find a new session for parent re-spawn");

    // ── Step 7: PM drops child 2 and creates replacement ─────────────
    harness
        .run_worker(
            pm_task_round2,
            vec![
                &format!("hydra issues update {child2_id} --status dropped"),
                &format!(
                    "hydra issues create 'Build search with SQLite FTS5' \
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

    // Find the replacement child issue.
    let all_issues = user.list_issues().await?;
    let child3 = find_issue_summary_by_description(&all_issues.issues, "SQLite FTS5")
        .context("replacement child should exist")?;
    let child3_id = child3.issue_id.clone();

    // Verify original child is still dropped.
    let child1_still_dropped = user.get_issue(&child1_id).await?;
    child1_still_dropped.assert_status(IssueStatus::Dropped);

    // Verify new child is open.
    let child3_check = user.get_issue(&child3_id).await?;
    child3_check.assert_status(IssueStatus::Open);

    // ── Step 8: SWE succeeds on replacement child and closes it ──────
    let swe_tasks_round2 = harness.await_sessions(&child3_id, 1).await?;
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
                &format!("hydra issues update {child3_id} --status closed"),
            ],
        )
        .await?;

    let child3_closed = user.get_issue(&child3_id).await?;
    child3_closed.assert_status(IssueStatus::Closed);

    // ── Step 9: PM re-spawns and closes parent ──────────────────────
    let pm_close_tasks = harness.await_sessions(&parent_id, 3).await?;
    assert_eq!(
        pm_close_tasks.len(),
        3,
        "parent should have three sessions after second re-spawn"
    );
    let pm_close_task = pm_close_tasks
        .iter()
        .find(|id| !pm_tasks_round2.contains(id))
        .expect("should find a new session for parent second re-spawn");

    harness
        .run_worker(
            pm_close_task,
            vec![&format!("hydra issues update {parent_id} --status closed")],
        )
        .await?;

    // ── Final verification ───────────────────────────────────────────
    let parent_final = user.get_issue(&parent_id).await?;
    parent_final.assert_status(IssueStatus::Closed);

    let child1_final = user.get_issue(&child1_id).await?;
    child1_final.assert_status(IssueStatus::Dropped);

    let child2_final = user.get_issue(&child2_id).await?;
    child2_final.assert_status(IssueStatus::Dropped);

    let child3_final = user.get_issue(&child3_id).await?;
    child3_final.assert_status(IssueStatus::Closed);

    // Verify children structure.
    let all_issues = user.list_issues().await?;
    parent_final.assert_has_child_with_status_in_summaries(
        &all_issues.issues,
        "Elasticsearch",
        IssueStatus::Dropped,
    );
    parent_final.assert_has_child_with_status_in_summaries(
        &all_issues.issues,
        "search result ranking",
        IssueStatus::Dropped,
    );
    parent_final.assert_has_child_with_status_in_summaries(
        &all_issues.issues,
        "SQLite FTS5",
        IssueStatus::Closed,
    );

    Ok(())
}
