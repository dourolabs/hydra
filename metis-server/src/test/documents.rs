use super::common::task_id;
use crate::{
    domain::jobs::{BundleSpec, Task},
    store::Status,
    test_utils::{
        spawn_test_server, spawn_test_server_with_state, test_client, test_state_handles,
    },
};
use chrono::Utc;
use metis_common::{
    DocumentId, TaskId,
    api::v1::documents::{
        Document, DocumentRecord, ListDocumentVersionsResponse, ListDocumentsResponse,
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
        None,
        None,
        HashMap::new(),
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
    let document = Document::new("Design doc".to_string(), "initial body".to_string())
        .with_path("docs/design.md");

    let created: UpsertDocumentResponse = client
        .post(format!("{}/v1/documents", server.base_url()))
        .json(&UpsertDocumentRequest::new(document.clone()))
        .send()
        .await?
        .json()
        .await?;

    assert!(!created.document_id.as_ref().is_empty());

    let fetched: DocumentRecord = client
        .get(format!(
            "{}/v1/documents/{}",
            server.base_url(),
            created.document_id
        ))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(fetched.id, created.document_id);
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
            .any(|record| record.id == created.document_id)
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
        .json(&UpsertDocumentRequest::new(Document::new(
            "Doc v1".to_string(),
            "body v1".to_string(),
        )))
        .send()
        .await?
        .json()
        .await?;

    let _updated: UpsertDocumentResponse = client
        .put(format!("{base}/v1/documents/{}", created.document_id))
        .json(&UpsertDocumentRequest::new(Document::new(
            "Doc v2".to_string(),
            "body v2".to_string(),
        )))
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
            Document::new("Doc".to_string(), "body".to_string())
                .with_created_by(missing_job.clone()),
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Non-running job also returns 400
    let handles = test_state_handles();
    let non_running = task_id("t-nonrunning");
    let task = sample_task(Status::Complete);
    handles
        .store
        .add_task_with_id(non_running.clone(), task, Utc::now())
        .await?;
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();
    let response = client
        .post(format!("{}/v1/documents", server.base_url()))
        .json(&UpsertDocumentRequest::new(
            Document::new("Doc".to_string(), "body".to_string())
                .with_created_by(non_running.clone()),
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Running job succeeds
    let handles = test_state_handles();
    let running_job = task_id("t-running");
    let task = sample_task(Status::Running);
    handles
        .store
        .add_task_with_id(running_job.clone(), task.clone(), Utc::now())
        .await?;
    handles.store.update_task(&running_job, task).await?;
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();
    let response = client
        .post(format!("{}/v1/documents", server.base_url()))
        .json(&UpsertDocumentRequest::new(
            Document::new("Doc".to_string(), "body".to_string())
                .with_created_by(running_job.clone()),
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    Ok(())
}

#[tokio::test]
async fn documents_support_search_filters() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let running_task = TaskId::new();
    let task = sample_task(Status::Running);
    handles
        .store
        .add_task_with_id(running_task.clone(), task.clone(), Utc::now())
        .await?;
    handles.store.update_task(&running_task, task).await?;
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();
    let base = server.base_url();

    let docs = [
        Document::new("Runbook".to_string(), "operations".to_string()).with_path("docs/runbook.md"),
        Document::new("API Guide".to_string(), "api details".to_string())
            .with_path("docs/guide.md"),
        Document::new("Notes".to_string(), "private".to_string())
            .with_path("notes/internal.md")
            .with_created_by(running_task.clone()),
    ];

    for doc in docs.iter() {
        let response = client
            .post(format!("{base}/v1/documents"))
            .json(&UpsertDocumentRequest::new(doc.clone()))
            .send()
            .await?;
        assert!(response.status().is_success());
    }

    let query = SearchDocumentsQuery::new(Some("runbook".to_string()), None, None, None);
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
            Some("docs/".to_string()),
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
        Document::new("Exact Doc".to_string(), "exact match".to_string())
            .with_path("docs/guide.md"),
        Document::new("Prefix Doc".to_string(), "prefix match".to_string())
            .with_path("docs/guide.md.bak"),
        Document::new("Nested Doc".to_string(), "nested match".to_string())
            .with_path("docs/guide.md/extra"),
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
            Some("docs/guide.md".to_string()),
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
            Some("docs/guide.md".to_string()),
            Some(true),
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
            Some("docs/guide.md".to_string()),
            Some(false),
            None,
        ))
        .send()
        .await?
        .json::<ListDocumentsResponse>()
        .await?;
    assert_eq!(by_prefix_explicit.documents.len(), 3);

    Ok(())
}
