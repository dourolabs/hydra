mod harness;

use anyhow::{Context, Result};
use harness::test_job_settings_full;
use metis_common::{
    issues::{IssueStatus, IssueType},
    patches::{PatchStatus, Review, UpsertPatchRequest},
};
use metis_server::domain::actors::ActorRef;
use std::str::FromStr;

/// Test that `metis patches merge <patch-id>` run as a worker rebases the patch
/// branch onto main, pushes the result to the remote, and marks the patch as
/// Merged.
#[tokio::test]
async fn worker_merge_pushes_to_remote() -> Result<()> {
    let repo_str = "octo/repo";
    let repo = metis_common::RepoName::from_str(repo_str)?;
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
        .create_job_for_issue(&repo, "merge the patch", &_parent_issue_id)
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
            vec![&format!("metis patches merge {patch_id_str}")],
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

    // ── 7. Verify the patch status was updated to Merged ─────────
    let final_patch = client.get_patch(&patch_id).await?;
    assert_eq!(
        final_patch.patch.status,
        PatchStatus::Merged,
        "patch should be marked as Merged"
    );

    Ok(())
}
