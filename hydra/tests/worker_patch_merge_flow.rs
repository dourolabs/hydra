mod harness;

use anyhow::{Context, Result};
use harness::test_job_settings_full;
use hydra_common::{
    issues::{IssueStatus, IssueType},
    patches::{PatchStatus, Review, UpsertPatchRequest},
};
use hydra_server::domain::actors::ActorRef;
use std::str::FromStr;

/// Test that `hydra patches merge <patch-id>` run as a worker squash merges the patch
/// branch onto main, pushes the result to the remote, and marks the patch as
/// Merged.
#[tokio::test]
async fn worker_merge_pushes_to_remote() -> Result<()> {
    let repo_str = "octo/repo";
    let repo = hydra_common::RepoName::from_str(repo_str)?;
    let feature_branch = "feature/my-change";

    let harness = harness::TestHarness::builder()
        .with_repo(repo_str)
        .build()
        .await?;

    let client = harness.client()?;

    // ── 1. Create a feature branch on the remote ─────────────────
    let feature_content = "initial content\nfeature change\n";
    harness
        .remote(repo_str)
        .create_branch(feature_branch, "README.md", feature_content)
        .context("failed to create feature branch")?;

    let main_head_before = harness.remote(repo_str).branch_sha("main")?;

    // ── 2. Create a parent issue with job settings ───────────────
    let _parent_issue_id = harness
        .default_user()
        .create_issue_with_settings(
            "merge task",
            IssueType::Task,
            IssueStatus::Open,
            Some("swe"),
            Some(test_job_settings_full(&repo, "worker:latest", "main")),
        )
        .await?;

    // ── 3. Create a patch with the feature branch name and an approving review ──
    let patch_id = harness
        .default_user()
        .create_patch("Merge feature", "A feature to merge", &repo)
        .await?;

    {
        let mut record = client.get_patch(&patch_id).await?;
        record.patch.branch_name = Some(feature_branch.to_string());
        record.patch.base_branch = Some("main".to_string());
        record.patch.reviews = vec![Review::new(
            "looks good".to_string(),
            true,
            "reviewer".to_string(),
            Some(chrono::Utc::now()),
        )];
        let request = UpsertPatchRequest::new(record.patch);
        client.update_patch(&patch_id, &request).await?;
    }

    // ── 4. Create a job for the issue and start it ─────────────────
    let job_id = harness
        .default_user()
        .create_session_for_issue(&repo, "merge the patch", &_parent_issue_id)
        .await?;

    harness
        .state()
        .start_pending_task(job_id.clone(), ActorRef::test())
        .await;

    harness
        .state()
        .transition_task_to_running(&job_id, ActorRef::test())
        .await
        .context("failed to transition task to running")?;

    let patch_id_str = patch_id.as_ref();
    let result = harness
        .run_worker(
            &job_id,
            vec![&format!("hydra patches merge {patch_id_str}")],
        )
        .await?;

    // ── 5. Verify the merge command succeeded ────────────────────
    assert_eq!(
        result.outputs.len(),
        1,
        "expected exactly one command output"
    );
    let merge_output = &result.outputs[0];
    assert_eq!(
        merge_output.status, 0,
        "merge command failed (exit code {}).\nstdout: {}\nstderr: {}",
        merge_output.status, merge_output.stdout, merge_output.stderr,
    );

    // ── 6. Verify the changes were pushed to main on the remote ──
    let main_head_after = harness.remote(repo_str).branch_sha("main")?;
    assert_ne!(
        main_head_after, main_head_before,
        "main branch should have advanced after merge"
    );

    let main_readme = harness.remote(repo_str).read_file("main", "README.md")?;
    assert_eq!(
        main_readme, feature_content,
        "main should contain the feature branch content after merge"
    );

    // ── 6b. Verify exactly 1 new commit on main (squash merge) ──
    let commit_count = harness
        .remote(repo_str)
        .commit_count(&main_head_before, &main_head_after)?;
    assert_eq!(
        commit_count, 1,
        "squash merge should produce exactly 1 new commit on main, got {commit_count}"
    );

    // ── 6c. Verify the squash commit message contains patch title and ID ──
    let commit_msg = harness.remote(repo_str).commit_message(&main_head_after)?;
    let patch_id_str = patch_id.as_ref();
    assert!(
        commit_msg.contains("Merge feature"),
        "squash commit message should contain the patch title, got: {commit_msg}"
    );
    assert!(
        commit_msg.contains(patch_id_str),
        "squash commit message should contain the patch ID, got: {commit_msg}"
    );

    // ── 7. Verify the patch status was updated to Merged ─────────
    let final_patch = client.get_patch(&patch_id).await?;
    assert_eq!(
        final_patch.patch.status,
        PatchStatus::Merged,
        "patch should be marked as Merged"
    );

    Ok(())
}

/// Test that `hydra patches merge` restores the original branch after a
/// successful merge. The worker starts on `hydra/{issue_id}/head`; after the
/// merge completes the working directory should be back on that branch, not on
/// the patch branch or the base branch.
#[tokio::test]
async fn worker_merge_restores_original_branch() -> Result<()> {
    let repo_str = "octo/repo";
    let repo = hydra_common::RepoName::from_str(repo_str)?;
    let feature_branch = "feature/branch-restore";

    let harness = harness::TestHarness::builder()
        .with_repo(repo_str)
        .build()
        .await?;

    let client = harness.client()?;

    // ── 1. Create a feature branch on the remote ─────────────────
    let feature_content = "initial content\nbranch restore change\n";
    harness
        .remote(repo_str)
        .create_branch(feature_branch, "README.md", feature_content)
        .context("failed to create feature branch")?;

    // ── 2. Create a parent issue with job settings ───────────────
    let parent_issue_id = harness
        .default_user()
        .create_issue_with_settings(
            "branch restore task",
            IssueType::Task,
            IssueStatus::Open,
            Some("swe"),
            Some(test_job_settings_full(&repo, "worker:latest", "main")),
        )
        .await?;

    // ── 3. Create a patch with the feature branch and an approving review ──
    let patch_id = harness
        .default_user()
        .create_patch("Branch restore", "Test branch restoration", &repo)
        .await?;

    {
        let mut record = client.get_patch(&patch_id).await?;
        record.patch.branch_name = Some(feature_branch.to_string());
        record.patch.base_branch = Some("main".to_string());
        record.patch.reviews = vec![Review::new(
            "lgtm".to_string(),
            true,
            "reviewer".to_string(),
            Some(chrono::Utc::now()),
        )];
        let request = UpsertPatchRequest::new(record.patch);
        client.update_patch(&patch_id, &request).await?;
    }

    // ── 4. Create a job for the issue and start it ─────────────────
    let job_id = harness
        .default_user()
        .create_session_for_issue(&repo, "merge and check branch", &parent_issue_id)
        .await?;

    harness
        .state()
        .start_pending_task(job_id.clone(), ActorRef::test())
        .await;

    harness
        .state()
        .transition_task_to_running(&job_id, ActorRef::test())
        .await
        .context("failed to transition task to running")?;

    // ── 5. Run the merge command followed by a branch check ──────
    let patch_id_str = patch_id.as_ref();
    let result = harness
        .run_worker(
            &job_id,
            vec![
                &format!("hydra patches merge {patch_id_str}"),
                "git rev-parse --abbrev-ref HEAD",
            ],
        )
        .await?;

    // ── 6. Verify both commands succeeded ────────────────────────
    assert_eq!(result.outputs.len(), 2, "expected two command outputs");

    let merge_output = &result.outputs[0];
    assert_eq!(
        merge_output.status, 0,
        "merge command failed (exit code {}).\nstdout: {}\nstderr: {}",
        merge_output.status, merge_output.stdout, merge_output.stderr,
    );

    let branch_output = &result.outputs[1];
    assert_eq!(
        branch_output.status, 0,
        "branch check command failed (exit code {}).\nstdout: {}\nstderr: {}",
        branch_output.status, branch_output.stdout, branch_output.stderr,
    );

    // ── 7. Verify the branch was restored ────────────────────────
    // The worker starts on hydra/{issue_id}/head. After the merge, the
    // working directory should be back on that branch.
    let current_branch = branch_output.stdout.trim();
    let issue_id_str = parent_issue_id.as_ref();
    let expected_branch = format!("hydra/{issue_id_str}/head");
    assert_eq!(
        current_branch, expected_branch,
        "after merge, expected to be on '{expected_branch}' but was on '{current_branch}'"
    );

    // ── 8. Sanity: the merge itself should have succeeded ────────
    let final_patch = client.get_patch(&patch_id).await?;
    assert_eq!(
        final_patch.patch.status,
        PatchStatus::Merged,
        "patch should be marked as Merged"
    );

    Ok(())
}

/// Test that two concurrent `hydra patches merge` commands both succeed
/// thanks to the retry logic that handles NotFastForward push errors.
///
/// Two workers each clone to separate temp dirs but share the same bare
/// remote. One worker's push succeeds first; the other's push fails with
/// NotFastForward, triggering the retry logic which should succeed.
#[tokio::test]
async fn concurrent_merges_both_succeed() -> Result<()> {
    let repo_str = "octo/repo";
    let repo = hydra_common::RepoName::from_str(repo_str)?;

    let harness = harness::TestHarness::builder()
        .with_repo(repo_str)
        .build()
        .await?;

    let client = harness.client()?;

    // ── 1. Create two feature branches, each modifying a different file ──
    let feature_branch_1 = "feature/concurrent-1";
    let feature_branch_2 = "feature/concurrent-2";
    let content_1 = "change from branch 1\n";
    let content_2 = "change from branch 2\n";

    harness
        .remote(repo_str)
        .create_branch(feature_branch_1, "file1.txt", content_1)
        .context("failed to create feature branch 1")?;
    harness
        .remote(repo_str)
        .create_branch(feature_branch_2, "file2.txt", content_2)
        .context("failed to create feature branch 2")?;

    let main_head_before = harness.remote(repo_str).branch_sha("main")?;

    // ── 2. Create two parent issues with job settings ────────────
    let parent_issue_1 = harness
        .default_user()
        .create_issue_with_settings(
            "merge task 1",
            IssueType::Task,
            IssueStatus::Open,
            Some("swe"),
            Some(test_job_settings_full(&repo, "worker:latest", "main")),
        )
        .await?;

    let parent_issue_2 = harness
        .default_user()
        .create_issue_with_settings(
            "merge task 2",
            IssueType::Task,
            IssueStatus::Open,
            Some("swe"),
            Some(test_job_settings_full(&repo, "worker:latest", "main")),
        )
        .await?;

    // ── 3. Create two patches with approved reviews ─────────────
    let patch_id_1 = harness
        .default_user()
        .create_patch("Concurrent merge 1", "First concurrent merge", &repo)
        .await?;
    let patch_id_2 = harness
        .default_user()
        .create_patch("Concurrent merge 2", "Second concurrent merge", &repo)
        .await?;

    for (patch_id, branch_name) in [
        (&patch_id_1, feature_branch_1),
        (&patch_id_2, feature_branch_2),
    ] {
        let mut record = client.get_patch(patch_id).await?;
        record.patch.branch_name = Some(branch_name.to_string());
        record.patch.base_branch = Some("main".to_string());
        record.patch.reviews = vec![Review::new(
            "approved".to_string(),
            true,
            "reviewer".to_string(),
            Some(chrono::Utc::now()),
        )];
        let request = UpsertPatchRequest::new(record.patch);
        client.update_patch(patch_id, &request).await?;
    }

    // ── 4. Create and start jobs for each patch ─────────────────
    let job_id_1 = harness
        .default_user()
        .create_session_for_issue(&repo, "merge patch 1", &parent_issue_1)
        .await?;
    let job_id_2 = harness
        .default_user()
        .create_session_for_issue(&repo, "merge patch 2", &parent_issue_2)
        .await?;

    for job_id in [&job_id_1, &job_id_2] {
        harness
            .state()
            .start_pending_task(job_id.clone(), ActorRef::test())
            .await;
        harness
            .state()
            .transition_task_to_running(job_id, ActorRef::test())
            .await
            .context("failed to transition task to running")?;
    }

    // ── 5. Run both workers concurrently ────────────────────────
    let cmd_1 = format!("hydra patches merge {}", patch_id_1.as_ref());
    let cmd_2 = format!("hydra patches merge {}", patch_id_2.as_ref());

    let (result_1, result_2) = tokio::join!(
        harness.run_worker(&job_id_1, vec![cmd_1.as_str()]),
        harness.run_worker(&job_id_2, vec![cmd_2.as_str()]),
    );

    // ── 6. Verify both merge commands succeeded ─────────────────
    let result_1 = result_1.context("worker 1 failed")?;
    let result_2 = result_2.context("worker 2 failed")?;

    assert_eq!(
        result_1.outputs.len(),
        1,
        "expected exactly one command output from worker 1"
    );
    assert_eq!(
        result_2.outputs.len(),
        1,
        "expected exactly one command output from worker 2"
    );

    let merge_output_1 = &result_1.outputs[0];
    let merge_output_2 = &result_2.outputs[0];
    assert_eq!(
        merge_output_1.status, 0,
        "merge 1 failed (exit code {}).\nstdout: {}\nstderr: {}",
        merge_output_1.status, merge_output_1.stdout, merge_output_1.stderr,
    );
    assert_eq!(
        merge_output_2.status, 0,
        "merge 2 failed (exit code {}).\nstdout: {}\nstderr: {}",
        merge_output_2.status, merge_output_2.stdout, merge_output_2.stderr,
    );

    // ── 7. Verify both patches are Merged ───────────────────────
    let final_patch_1 = client.get_patch(&patch_id_1).await?;
    let final_patch_2 = client.get_patch(&patch_id_2).await?;
    assert_eq!(
        final_patch_1.patch.status,
        PatchStatus::Merged,
        "patch 1 should be marked as Merged"
    );
    assert_eq!(
        final_patch_2.patch.status,
        PatchStatus::Merged,
        "patch 2 should be marked as Merged"
    );

    // ── 8. Verify the remote main branch contains both changes ──
    let main_head_after = harness.remote(repo_str).branch_sha("main")?;
    assert_ne!(
        main_head_after, main_head_before,
        "main branch should have advanced after merges"
    );

    let main_file1 = harness.remote(repo_str).read_file("main", "file1.txt")?;
    let main_file2 = harness.remote(repo_str).read_file("main", "file2.txt")?;
    assert_eq!(
        main_file1, content_1,
        "main should contain file1.txt from branch 1"
    );
    assert_eq!(
        main_file2, content_2,
        "main should contain file2.txt from branch 2"
    );

    // ── 9. Verify exactly 2 squash commits on main (one per merge) ──
    let commit_count = harness
        .remote(repo_str)
        .commit_count(&main_head_before, &main_head_after)?;
    assert_eq!(
        commit_count, 2,
        "two concurrent squash merges should produce exactly 2 new commits on main, got {commit_count}"
    );

    Ok(())
}
