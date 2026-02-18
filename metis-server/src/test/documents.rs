use crate::{
    domain::{
        actors::ActorRef,
        jobs::{BundleSpec, Task},
        users::Username,
    },
    store::Status,
    test_utils::{
        spawn_test_server, spawn_test_server_with_state, test_client, test_state_handles,
    },
};
use chrono::Utc;
use metis_common::{
    DocumentId, TaskId,
    api::v1::documents::{
        Document, DocumentVersionRecord, ListDocumentVersionsResponse, ListDocumentsResponse,
        SearchDocumentsQuery, UpsertDocumentRequest, UpsertDocumentResponse,
    },
};
use reqwest::StatusCode;
use std::collections::HashMap;

fn sample_task(status: Status) -> Task {
    let mut task = Task::new(
        "prompt".to_string(),
        BundleSpec::None,
        None,
        Username::from("test-creator"),
        None,
        None,
        HashMap::new(),
        None,
        None,
        None,
    );
    task.status = status;
    task
}

#[tokio::test]
async fn documents_can_be_created_listed_and_retrieved() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let document = Document::new(
        "Design doc".to_string(),
        "initial body".to_string(),
        Some("docs/design.md".to_string()),
        None,
        false,
    )
    .unwrap();

    let created: UpsertDocumentResponse = client
        .post(format!("{}/v1/documents", server.base_url()))
        .json(&UpsertDocumentRequest::new(document.clone()))
        .send()
        .await?
        .json()
        .await?;

    assert!(!created.document_id.as_ref().is_empty());

    let fetched: DocumentVersionRecord = client
        .get(format!(
            "{}/v1/documents/{}",
            server.base_url(),
            created.document_id
        ))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(fetched.document_id, created.document_id);
    assert_eq!(fetched.document, document);

    let list: ListDocumentsResponse = client
        .get(format!("{}/v1/documents", server.base_url()))
        .send()
        .await?
        .json()
        .await?;
    assert!(
        list.documents
            .iter()
            .any(|record| record.document_id == created.document_id)
    );

    Ok(())
}

#[tokio::test]
async fn document_versions_endpoints_return_history() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    let created: UpsertDocumentResponse = client
        .post(format!("{base}/v1/documents"))
        .json(&UpsertDocumentRequest::new(
            Document::new(
                "Doc v1".to_string(),
                "body v1".to_string(),
                None,
                None,
                false,
            )
            .unwrap(),
        ))
        .send()
        .await?
        .json()
        .await?;

    let _updated: UpsertDocumentResponse = client
        .put(format!("{base}/v1/documents/{}", created.document_id))
        .json(&UpsertDocumentRequest::new(
            Document::new(
                "Doc v2".to_string(),
                "body v2".to_string(),
                None,
                None,
                false,
            )
            .unwrap(),
        ))
        .send()
        .await?
        .json()
        .await?;

    let versions: ListDocumentVersionsResponse = client
        .get(format!(
            "{base}/v1/documents/{}/versions",
            created.document_id
        ))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(versions.versions.len(), 2);
    assert_eq!(versions.versions[0].document_id, created.document_id);
    assert_eq!(versions.versions[0].version, 1);
    assert_eq!(versions.versions[0].document.title, "Doc v1");
    assert_eq!(versions.versions[1].version, 2);
    assert_eq!(versions.versions[1].document.title, "Doc v2");

    let version = client
        .get(format!(
            "{base}/v1/documents/{}/versions/2",
            created.document_id
        ))
        .send()
        .await?;
    assert!(version.status().is_success());

    Ok(())
}

#[tokio::test]
async fn documents_require_running_task_for_created_by() -> anyhow::Result<()> {
    // Missing job returns 400
    let server = spawn_test_server().await?;
    let client = test_client();
    let missing_job = TaskId::new();
    let response = client
        .post(format!("{}/v1/documents", server.base_url()))
        .json(&UpsertDocumentRequest::new(
            Document::new(
                "Doc".to_string(),
                "body".to_string(),
                None,
                Some(missing_job.clone()),
                false,
            )
            .unwrap(),
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Non-running job also returns 400
    let handles = test_state_handles();
    let task = sample_task(Status::Created);
    let (non_running, _) = handles
        .store
        .add_task(task, Utc::now(), &ActorRef::test())
        .await?;
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();
    let response = client
        .post(format!("{}/v1/documents", server.base_url()))
        .json(&UpsertDocumentRequest::new(
            Document::new(
                "Doc".to_string(),
                "body".to_string(),
                None,
                Some(non_running.clone()),
                false,
            )
            .unwrap(),
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Running job succeeds
    let handles = test_state_handles();
    let task = sample_task(Status::Running);
    let (running_job, _) = handles
        .store
        .add_task(task.clone(), Utc::now(), &ActorRef::test())
        .await?;
    handles
        .store
        .update_task(&running_job, task, &ActorRef::test())
        .await?;
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();
    let response = client
        .post(format!("{}/v1/documents", server.base_url()))
        .json(&UpsertDocumentRequest::new(
            Document::new(
                "Doc".to_string(),
                "body".to_string(),
                None,
                Some(running_job.clone()),
                false,
            )
            .unwrap(),
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    Ok(())
}

#[tokio::test]
async fn documents_support_search_filters() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let task = sample_task(Status::Running);
    let (running_task, _) = handles
        .store
        .add_task(task.clone(), Utc::now(), &ActorRef::test())
        .await?;
    handles
        .store
        .update_task(&running_task, task, &ActorRef::test())
        .await?;
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();
    let base = server.base_url();

    let docs = [
        Document::new(
            "Runbook".to_string(),
            "operations".to_string(),
            Some("docs/runbook.md".to_string()),
            None,
            false,
        )
        .unwrap(),
        Document::new(
            "API Guide".to_string(),
            "api details".to_string(),
            Some("docs/guide.md".to_string()),
            None,
            false,
        )
        .unwrap(),
        Document::new(
            "Notes".to_string(),
            "private".to_string(),
            Some("notes/internal.md".to_string()),
            Some(running_task.clone()),
            false,
        )
        .unwrap(),
    ];

    for doc in docs.iter() {
        let response = client
            .post(format!("{base}/v1/documents"))
            .json(&UpsertDocumentRequest::new(doc.clone()))
            .send()
            .await?;
        assert!(response.status().is_success());
    }

    let query = SearchDocumentsQuery::new(Some("runbook".to_string()), None, None, None, None);
    let matching = client
        .get(format!("{base}/v1/documents"))
        .query(&query)
        .send()
        .await?
        .json::<ListDocumentsResponse>()
        .await?;
    assert_eq!(matching.documents.len(), 1);
    assert_eq!(matching.documents[0].document.title, "Runbook");

    let by_path = client
        .get(format!("{base}/v1/documents"))
        .query(&SearchDocumentsQuery::new(
            None,
            Some("/docs/".to_string()),
            None,
            None,
            None,
        ))
        .send()
        .await?
        .json::<ListDocumentsResponse>()
        .await?;
    assert_eq!(by_path.documents.len(), 2);

    let by_creator = client
        .get(format!("{base}/v1/documents"))
        .query(&SearchDocumentsQuery::new(
            None,
            None,
            None,
            Some(running_task.clone()),
            None,
        ))
        .send()
        .await?
        .json::<ListDocumentsResponse>()
        .await?;
    assert_eq!(by_creator.documents.len(), 1);
    assert_eq!(by_creator.documents[0].document.title, "Notes");

    Ok(())
}

#[tokio::test]
async fn documents_missing_resources_return_404() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let missing = DocumentId::new();

    let response = client
        .get(format!("{}/v1/documents/{}", server.base_url(), missing))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let response = client
        .get(format!(
            "{}/v1/documents/{}/versions",
            server.base_url(),
            missing
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn documents_support_exact_path_matching() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    let docs = [
        Document::new(
            "Exact Doc".to_string(),
            "exact match".to_string(),
            Some("docs/guide.md".to_string()),
            None,
            false,
        )
        .unwrap(),
        Document::new(
            "Prefix Doc".to_string(),
            "prefix match".to_string(),
            Some("docs/guide.md.bak".to_string()),
            None,
            false,
        )
        .unwrap(),
        Document::new(
            "Nested Doc".to_string(),
            "nested match".to_string(),
            Some("docs/guide.md/extra".to_string()),
            None,
            false,
        )
        .unwrap(),
    ];

    for doc in docs.iter() {
        let response = client
            .post(format!("{base}/v1/documents"))
            .json(&UpsertDocumentRequest::new(doc.clone()))
            .send()
            .await?;
        assert!(response.status().is_success());
    }

    // Without path_is_exact, prefix matching returns all 3 docs
    let by_prefix = client
        .get(format!("{base}/v1/documents"))
        .query(&SearchDocumentsQuery::new(
            None,
            Some("/docs/guide.md".to_string()),
            None,
            None,
            None,
        ))
        .send()
        .await?
        .json::<ListDocumentsResponse>()
        .await?;
    assert_eq!(by_prefix.documents.len(), 3);

    // With path_is_exact=true, only exact match is returned
    let by_exact = client
        .get(format!("{base}/v1/documents"))
        .query(&SearchDocumentsQuery::new(
            None,
            Some("/docs/guide.md".to_string()),
            Some(true),
            None,
            None,
        ))
        .send()
        .await?
        .json::<ListDocumentsResponse>()
        .await?;
    assert_eq!(by_exact.documents.len(), 1);
    assert_eq!(by_exact.documents[0].document.title, "Exact Doc");

    // With path_is_exact=false, prefix matching is used (default behavior)
    let by_prefix_explicit = client
        .get(format!("{base}/v1/documents"))
        .query(&SearchDocumentsQuery::new(
            None,
            Some("/docs/guide.md".to_string()),
            Some(false),
            None,
            None,
        ))
        .send()
        .await?
        .json::<ListDocumentsResponse>()
        .await?;
    assert_eq!(by_prefix_explicit.documents.len(), 3);

    Ok(())
}

// ===== Deletion Tests =====

#[tokio::test]
async fn delete_document_basic_operation() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    // Create a document
    let document = Document::new(
        "Doc to delete".to_string(),
        "document body".to_string(),
        None,
        None,
        false,
    )
    .unwrap();

    let created: UpsertDocumentResponse = client
        .post(format!("{base}/v1/documents"))
        .json(&UpsertDocumentRequest::new(document))
        .send()
        .await?
        .json()
        .await?;

    // Delete the document
    let deleted: DocumentVersionRecord = client
        .delete(format!("{base}/v1/documents/{}", created.document_id))
        .send()
        .await?
        .json()
        .await?;

    // Verify the response has deleted=true
    assert!(deleted.document.deleted);

    // Verify listing excludes the deleted document
    let list: ListDocumentsResponse = client
        .get(format!("{base}/v1/documents"))
        .send()
        .await?
        .json()
        .await?;

    assert!(
        !list
            .documents
            .iter()
            .any(|d| d.document_id == created.document_id)
    );

    Ok(())
}

#[tokio::test]
async fn delete_document_include_deleted_in_listing() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    // Create and delete a document
    let document = Document::new(
        "Deleted doc".to_string(),
        "document body".to_string(),
        None,
        None,
        false,
    )
    .unwrap();

    let created: UpsertDocumentResponse = client
        .post(format!("{base}/v1/documents"))
        .json(&UpsertDocumentRequest::new(document))
        .send()
        .await?
        .json()
        .await?;

    client
        .delete(format!("{base}/v1/documents/{}", created.document_id))
        .send()
        .await?
        .error_for_status()?;

    // List without include_deleted - verify not present
    let list_without: ListDocumentsResponse = client
        .get(format!("{base}/v1/documents"))
        .send()
        .await?
        .json()
        .await?;

    assert!(
        !list_without
            .documents
            .iter()
            .any(|d| d.document_id == created.document_id)
    );

    // List with include_deleted=true - verify present with deleted=true
    let list_with: ListDocumentsResponse = client
        .get(format!("{base}/v1/documents"))
        .query(&SearchDocumentsQuery::new(
            None,
            None,
            None,
            None,
            Some(true),
        ))
        .send()
        .await?
        .json()
        .await?;

    let deleted_doc = list_with
        .documents
        .iter()
        .find(|d| d.document_id == created.document_id);

    assert!(deleted_doc.is_some());
    assert!(deleted_doc.unwrap().document.deleted);

    Ok(())
}

#[tokio::test]
async fn delete_document_get_deleted_by_id() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    // Create and delete a document
    let document = Document::new(
        "Get deleted doc".to_string(),
        "document body".to_string(),
        None,
        None,
        false,
    )
    .unwrap();

    let created: UpsertDocumentResponse = client
        .post(format!("{base}/v1/documents"))
        .json(&UpsertDocumentRequest::new(document))
        .send()
        .await?
        .json()
        .await?;

    client
        .delete(format!("{base}/v1/documents/{}", created.document_id))
        .send()
        .await?
        .error_for_status()?;

    // GET by ID should return 404 for deleted documents
    let response = client
        .get(format!("{base}/v1/documents/{}", created.document_id))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn delete_document_idempotency() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    // Create and delete a document
    let document = Document::new(
        "Idempotency doc".to_string(),
        "document body".to_string(),
        None,
        None,
        false,
    )
    .unwrap();

    let created: UpsertDocumentResponse = client
        .post(format!("{base}/v1/documents"))
        .json(&UpsertDocumentRequest::new(document))
        .send()
        .await?
        .json()
        .await?;

    // First delete
    let first_delete = client
        .delete(format!("{base}/v1/documents/{}", created.document_id))
        .send()
        .await?;

    assert!(first_delete.status().is_success());

    // Second delete - should return 200 (idempotent)
    let second_delete = client
        .delete(format!("{base}/v1/documents/{}", created.document_id))
        .send()
        .await?;

    assert!(second_delete.status().is_success());

    Ok(())
}

#[tokio::test]
async fn delete_document_non_existent() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Attempt to delete non-existent ID
    let missing = DocumentId::new();
    let response = client
        .delete(format!("{}/v1/documents/{}", server.base_url(), missing))
        .send()
        .await?;

    // Verify 404 response
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

// ===== Negative Version Offset Tests =====

#[tokio::test]
async fn get_document_version_negative_offset_returns_correct_version() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    // Create document (v1)
    let created: UpsertDocumentResponse = client
        .post(format!("{base}/v1/documents"))
        .json(&UpsertDocumentRequest::new(
            Document::new(
                "Doc v1".to_string(),
                "body v1".to_string(),
                None,
                None,
                false,
            )
            .unwrap(),
        ))
        .send()
        .await?
        .json()
        .await?;

    // Update document (v2)
    client
        .put(format!("{base}/v1/documents/{}", created.document_id))
        .json(&UpsertDocumentRequest::new(
            Document::new(
                "Doc v2".to_string(),
                "body v2".to_string(),
                None,
                None,
                false,
            )
            .unwrap(),
        ))
        .send()
        .await?
        .error_for_status()?;

    // version=-1 should return v1 (second-to-last)
    let v_minus_1: DocumentVersionRecord = client
        .get(format!(
            "{base}/v1/documents/{}/versions/-1",
            created.document_id
        ))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(v_minus_1.version, 1);
    assert_eq!(v_minus_1.document.title, "Doc v1");

    // version=0 should return 400
    let response = client
        .get(format!(
            "{base}/v1/documents/{}/versions/0",
            created.document_id
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // version=-100 should return 400 (out of range)
    let response = client
        .get(format!(
            "{base}/v1/documents/{}/versions/-100",
            created.document_id
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    Ok(())
}
