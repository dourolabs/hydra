mod harness;

use anyhow::Result;
use std::str::FromStr;

/// Verifies the document store sync/push workflow through a worker.
///
/// Exercises the full lifecycle:
///   1. A worker creates a document via the server API.
///   2. A second worker runs `metis documents sync` (no directory argument,
///      relying on the `METIS_DOCUMENTS_DIR` env var set by `worker_run`).
///   3. The worker verifies the synced file exists, edits it in-place.
///   4. The worker runs `metis documents push` (no directory argument).
///   5. The test verifies the server document has the updated content.
///   6. The test verifies documents were NOT auto-committed to the git repo.
#[tokio::test]
async fn document_sync_push_through_worker() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/doc-sync")
        .build()
        .await?;
    let user = harness.default_user();
    let repo = metis_common::RepoName::from_str("acme/doc-sync")?;

    // ── Phase 1: Create a document via a worker CLI command ─────────
    let phase1_issue = user.create_issue("create initial document").await?;
    let phase1_job = user
        .create_session_for_issue(&repo, "create document", &phase1_issue)
        .await?;

    harness
        .run_worker(
            &phase1_job,
            vec![
                "metis documents create --title \"Sync Test Doc\" --path \"notes/sync-test.md\" --body \"original content\"",
            ],
        )
        .await?;

    // Verify the document was created on the server.
    let client = harness.client()?;
    let doc = client
        .get_document_by_path("/notes/sync-test.md", false)
        .await?;
    assert_eq!(doc.document.title, "Sync Test Doc");
    assert!(doc.document.body_markdown.contains("original content"));
    let doc_id = doc.document_id.clone();

    // ── Phase 2: Sync, edit, and push via a second worker ───────────
    let phase2_issue = user.create_issue("sync edit and push document").await?;
    let phase2_job = user
        .create_session_for_issue(&repo, "sync and push document", &phase2_issue)
        .await?;

    harness
        .run_worker(
            &phase2_job,
            vec![
                // Sync documents (no directory arg — uses METIS_DOCUMENTS_DIR).
                "metis documents sync",
                // Verify the file was synced locally.
                "test -f $METIS_DOCUMENTS_DIR/notes/sync-test.md",
                // Edit the file in-place with new content.
                "echo 'updated content from worker' > $METIS_DOCUMENTS_DIR/notes/sync-test.md",
                // Push documents back (no directory arg — uses METIS_DOCUMENTS_DIR).
                "metis documents push",
            ],
        )
        .await?;

    // ── Verify: server document has updated content ─────────────────
    let updated_doc = client.get_document(&doc_id, false).await?;
    assert!(
        updated_doc
            .document
            .body_markdown
            .contains("updated content from worker"),
        "expected server document to contain 'updated content from worker', got: {}",
        updated_doc.document.body_markdown,
    );

    // ── Verify: documents are NOT in the auto-committed git repo ────
    let remote = harness.remote("acme/doc-sync");
    let issue_head = format!("metis/{phase2_issue}/head");
    assert!(
        remote.branch_exists(&issue_head),
        "expected branch '{issue_head}' to exist in the remote"
    );

    let diff = remote.diff("main", &issue_head)?;
    assert!(
        !diff.contains("notes/sync-test.md"),
        "document file should NOT appear in the git diff, but found it in:\n{diff}"
    );
    assert!(
        !diff.contains("documents/"),
        "no documents/ path should appear in the git diff, but found it in:\n{diff}"
    );

    Ok(())
}
