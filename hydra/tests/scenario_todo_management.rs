mod harness;

use anyhow::Result;
use harness::IssueAssertions;
use hydra_common::issues::IssueStatus;
use std::str::FromStr;

/// Scenario 8: Todo list management through a worker.
///
/// Exercises the complete todo lifecycle via CLI commands executed by a
/// worker:
///   1. Add 3 todo items → verify 3 todos, all incomplete
///   2. Mark one done → verify 1/3 done
///   3. Replace entire todo list → verify new list applied
///   4. Mark all done and close issue → verify closed
#[tokio::test]
async fn todo_management_through_worker() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/todo-mgmt")
        .build()
        .await?;
    let user = harness.default_user();
    let repo = hydra_common::RepoName::from_str("acme/todo-mgmt")?;

    // Create the target issue.
    let target_id = user.create_issue("todo management test").await?;

    // Phase 1: Add 3 todo items via worker CLI.
    let phase1_issue = user.create_issue("phase1 worker").await?;
    let phase1_job = user
        .create_session_for_issue(&repo, "add todos", &phase1_issue)
        .await?;

    harness
        .run_worker(
            &phase1_job,
            vec![
                &format!("metis issues todo {target_id} --add \"Research codebase\""),
                &format!("metis issues todo {target_id} --add \"Write design doc\""),
                &format!("metis issues todo {target_id} --add \"Create review issue\""),
            ],
        )
        .await?;

    // Verify: 3 todos, all incomplete.
    let issue = user.get_issue(&target_id).await?;
    issue.assert_todo_count(3);
    assert!(
        issue.issue.todo_list.iter().all(|t| !t.is_done),
        "all todos should be incomplete after adding"
    );
    assert_eq!(issue.issue.todo_list[0].description, "Research codebase");
    assert_eq!(issue.issue.todo_list[1].description, "Write design doc");
    assert_eq!(issue.issue.todo_list[2].description, "Create review issue");

    // Phase 2: Mark one todo done.
    let phase2_issue = user.create_issue("phase2 worker").await?;
    let phase2_job = user
        .create_session_for_issue(&repo, "mark todo done", &phase2_issue)
        .await?;

    harness
        .run_worker(
            &phase2_job,
            vec![&format!("metis issues todo {target_id} --done 1")],
        )
        .await?;

    // Verify: 1 of 3 done.
    let issue = user.get_issue(&target_id).await?;
    issue.assert_todo_count(3);
    let done_count = issue.issue.todo_list.iter().filter(|t| t.is_done).count();
    assert_eq!(done_count, 1, "expected 1 done todo, got {done_count}");
    assert!(
        issue.issue.todo_list[0].is_done,
        "first todo should be done"
    );

    // Phase 3: Replace entire todo list with new items.
    let phase3_issue = user.create_issue("phase3 worker").await?;
    let phase3_job = user
        .create_session_for_issue(&repo, "replace todos", &phase3_issue)
        .await?;

    harness
        .run_worker(
            &phase3_job,
            vec![&format!(
                "metis issues todo {target_id} --replace \"[x] Investigate bug\" \"Fix root cause\" \"Add regression test\""
            )],
        )
        .await?;

    // Verify: new todo list is applied.
    let issue = user.get_issue(&target_id).await?;
    issue.assert_todo_count(3);
    assert_eq!(issue.issue.todo_list[0].description, "Investigate bug");
    assert!(
        issue.issue.todo_list[0].is_done,
        "first replaced todo should be done (prefixed with [x])"
    );
    assert_eq!(issue.issue.todo_list[1].description, "Fix root cause");
    assert!(
        !issue.issue.todo_list[1].is_done,
        "second replaced todo should be incomplete"
    );
    assert_eq!(issue.issue.todo_list[2].description, "Add regression test");
    assert!(
        !issue.issue.todo_list[2].is_done,
        "third replaced todo should be incomplete"
    );

    // Phase 4: Mark all done and close issue.
    let phase4_issue = user.create_issue("phase4 worker").await?;
    let phase4_job = user
        .create_session_for_issue(&repo, "close issue", &phase4_issue)
        .await?;

    harness
        .run_worker(
            &phase4_job,
            vec![
                &format!("metis issues todo {target_id} --done 2"),
                &format!("metis issues todo {target_id} --done 3"),
                &format!("metis issues update {target_id} --status closed"),
            ],
        )
        .await?;

    // Verify: issue is closed and all todos are done.
    let issue = user.get_issue(&target_id).await?;
    issue.assert_status(IssueStatus::Closed);
    assert!(
        issue.issue.todo_list.iter().all(|t| t.is_done),
        "all todos should be done after closing"
    );

    Ok(())
}
