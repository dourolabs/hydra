mod harness;

use anyhow::Result;
use harness::IssueSummaryAssertions;
use metis_common::issues::IssueStatus;
use std::str::FromStr;

/// Scenario 10: Document operations through a worker.
///
/// Exercises the document lifecycle via CLI commands executed by a worker:
///   1. PM worker creates a document via CLI → verify document exists
///   2. PM worker updates the document → verify 2 versions
///   3. PM worker creates a review child issue referencing the document path
///      → verify child created
#[tokio::test]
async fn document_operations_through_worker() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/doc-ops")
        .build()
        .await?;
    let user = harness.default_user();
    let repo = metis_common::RepoName::from_str("acme/doc-ops")?;

    // Create the parent issue for the PM agent.
    let parent_id = user.create_issue("PM: design feature X").await?;

    // Phase 1: PM worker creates a document via CLI.
    let phase1_issue = user.create_issue("phase1 doc worker").await?;
    let phase1_job = user
        .create_job_for_issue(&repo, "create document", &phase1_issue)
        .await?;

    harness
        .run_worker(
            &phase1_job,
            vec![
                "metis --output-format jsonl documents create --title \"Design for feature X\" --path \"designs/feature-x.md\" --body \"# Feature X\\n\\nInitial design draft.\" | tee doc_output.txt",
            ],
        )
        .await?;

    // Verify: document exists with correct title and body.
    // Use the client API to list documents.
    let client = harness.client()?;
    let query = metis_common::documents::SearchDocumentsQuery::new(
        Some("feature X".to_string()),
        None,
        None,
        None,
        None,
    );
    let docs = client.list_documents(&query).await?;

    assert!(
        !docs.documents.is_empty(),
        "expected at least one document matching 'feature X'"
    );
    let doc = &docs.documents[0];
    assert_eq!(doc.document.title, "Design for feature X");
    assert!(doc.document.body_markdown.contains("Initial design draft"));
    assert_eq!(doc.document.path.as_deref(), Some("/designs/feature-x.md"));
    let doc_id = doc.document_id.clone();

    // Phase 2: PM worker updates the document with revised content.
    let phase2_issue = user.create_issue("phase2 doc worker").await?;
    let phase2_job = user
        .create_job_for_issue(&repo, "update document", &phase2_issue)
        .await?;

    harness
        .run_worker(
            &phase2_job,
            vec![&format!(
                "metis documents update {doc_id} --body \"# Feature X\\n\\nRevised design with implementation details.\\n\\n## Architecture\\n\\nComponent-based approach.\""
            )],
        )
        .await?;

    // Verify: document has been updated (we can check latest version content).
    let updated_doc = client.get_document(&doc_id, false).await?;
    assert!(
        updated_doc
            .document
            .body_markdown
            .contains("Revised design"),
        "document body should contain revised content"
    );
    // Verify version is at least 2.
    assert!(
        updated_doc.version >= 2,
        "expected at least 2 versions, got {}",
        updated_doc.version
    );

    // Phase 3: PM worker creates a review child issue referencing the document.
    let phase3_issue = user.create_issue("phase3 doc worker").await?;
    let phase3_job = user
        .create_job_for_issue(&repo, "create review issue", &phase3_issue)
        .await?;

    harness
        .run_worker(
            &phase3_job,
            vec![&format!(
                "metis issues create --deps child-of:{parent_id} \"Review design document at designs/feature-x.md\""
            )],
        )
        .await?;

    // Verify: review child issue was created under the parent.
    let issues = user.list_issues().await?;
    let parent = issues
        .issues
        .iter()
        .find(|i| i.issue_id == parent_id)
        .expect("parent issue should exist");

    parent.assert_has_child_with_status(
        &issues.issues,
        "Review design document",
        IssueStatus::Open,
    );

    Ok(())
}
