mod harness;

use anyhow::Result;
use metis_common::api::v1::relations::ListRelationsRequest;
use std::str::FromStr;

/// Validates the has-document relation auto-creation flow.
///
/// When METIS_ISSUE_ID is set and documents are created, updated, or pushed
/// via the CLI, a has-document relation should be automatically created
/// linking the issue to the document.
#[tokio::test]
async fn has_document_relation_auto_linking() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/doc-rel")
        .build()
        .await?;
    let user = harness.default_user();
    let repo = metis_common::RepoName::from_str("acme/doc-rel")?;
    let client = harness.client()?;

    // ── Phase 1: documents create auto-links the issue ──────────────
    let issue1 = user.create_issue("test doc create linking").await?;
    let job1 = user
        .create_session_for_issue(&repo, "create doc", &issue1)
        .await?;

    harness
        .run_worker(
            &job1,
            vec![
                "metis documents create --title \"Linked Doc\" --path \"docs/linked.md\" --body \"hello\"",
            ],
        )
        .await?;

    // Fetch the created document to get its ID.
    let doc = client
        .get_document_by_path("/docs/linked.md", false)
        .await?;
    let doc_id = doc.document_id.clone();

    // Assert: has-document relation exists from issue to document.
    let relations = client
        .list_relations(&ListRelationsRequest {
            source_id: Some(issue1.clone().into()),
            rel_type: Some("has-document".to_string()),
            ..Default::default()
        })
        .await?;
    assert_eq!(
        relations.relations.len(),
        1,
        "expected exactly one has-document relation after create"
    );
    assert_eq!(
        relations.relations[0].target_id,
        doc_id.clone().into(),
        "relation target should be the document"
    );

    // ── Phase 2: documents update is idempotent (no duplicate) ──────
    let issue2_job = user
        .create_session_for_issue(&repo, "update doc", &issue1)
        .await?;

    harness
        .run_worker(
            &issue2_job,
            vec![&format!(
                "metis documents update {doc_id} --title \"Linked Doc v2\" --body \"updated\""
            )],
        )
        .await?;

    // Assert: still only one has-document relation (idempotent).
    let relations = client
        .list_relations(&ListRelationsRequest {
            source_id: Some(issue1.clone().into()),
            rel_type: Some("has-document".to_string()),
            ..Default::default()
        })
        .await?;
    assert_eq!(
        relations.relations.len(),
        1,
        "expected exactly one has-document relation after update (idempotent)"
    );

    // ── Phase 3: documents push auto-links new documents ────────────
    let issue3 = user.create_issue("test doc push linking").await?;
    let job3 = user
        .create_session_for_issue(&repo, "push doc", &issue3)
        .await?;

    harness
        .run_worker(
            &job3,
            vec![
                // Sync documents (sets up METIS_DOCUMENTS_DIR).
                "metis documents sync",
                // Create a new file locally.
                "mkdir -p $METIS_DOCUMENTS_DIR/docs && echo 'push content' > $METIS_DOCUMENTS_DIR/docs/pushed.md",
                // Push documents back.
                "metis documents push",
            ],
        )
        .await?;

    // Fetch the pushed document to get its ID.
    let pushed_doc = client
        .get_document_by_path("/docs/pushed.md", false)
        .await?;
    let pushed_doc_id = pushed_doc.document_id.clone();

    // Assert: has-document relation exists from issue3 to pushed document.
    let relations = client
        .list_relations(&ListRelationsRequest {
            source_id: Some(issue3.clone().into()),
            rel_type: Some("has-document".to_string()),
            ..Default::default()
        })
        .await?;
    assert!(
        relations
            .relations
            .iter()
            .any(|r| r.target_id == pushed_doc_id.clone().into()),
        "expected has-document relation from issue3 to pushed document"
    );

    // ── Phase 4: no relation when METIS_ISSUE_ID is unset ───────────
    let issue4 = user.create_issue("test no linking").await?;
    let job4 = user
        .create_session_for_issue(&repo, "no link doc", &issue4)
        .await?;

    // Count relations before.
    let all_before = client
        .list_relations(&ListRelationsRequest {
            rel_type: Some("has-document".to_string()),
            ..Default::default()
        })
        .await?;
    let count_before = all_before.relations.len();

    // Run a create without --issue-id and without METIS_ISSUE_ID by
    // unsetting the env var explicitly.
    harness
        .run_worker(
            &job4,
            vec![
                "unset METIS_ISSUE_ID && metis documents create --title \"Unlinked Doc\" --path \"docs/unlinked.md\" --body \"no link\"",
            ],
        )
        .await?;

    // Assert: no new has-document relation was created.
    let all_after = client
        .list_relations(&ListRelationsRequest {
            rel_type: Some("has-document".to_string()),
            ..Default::default()
        })
        .await?;
    assert_eq!(
        all_after.relations.len(),
        count_before,
        "expected no new has-document relation when METIS_ISSUE_ID is unset"
    );

    Ok(())
}
