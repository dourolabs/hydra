use crate::{
    domain::{
        actors::{Actor, ActorRef},
        sessions::Session,
        users::Username,
    },
    store::Status,
    test_utils::{
        spawn_test_server, spawn_test_server_with_state, test_client, test_state_handles,
    },
};
use chrono::Utc;
use hydra_common::{
    ActorId, DocumentId, SessionId,
    api::v1::documents::{
        Document, DocumentVersionRecord, ListDocumentPathsResponse, ListDocumentVersionsResponse,
        ListDocumentsResponse, SearchDocumentsQuery, UpsertDocumentRequest, UpsertDocumentResponse,
    },
};
use reqwest::{Client, StatusCode, header};
use std::collections::HashMap;

fn sample_task(status: Status) -> Session {
    use crate::domain::sessions::{AgentConfig, SessionMode};
    use crate::routes::sessions::mount_spec_from_create_request;
    Session::new(
        Username::from("test-creator"),
        None,
        None,
        AgentConfig::default(),
        mount_spec_from_create_request(hydra_common::api::v1::sessions::Bundle::None, None),
        None,
        HashMap::new(),
        None,
        None,
        None,
        SessionMode::Headless,
        status,
        None,
        None,
    )
}

#[tokio::test]
async fn documents_can_be_created_listed_and_retrieved() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let document = Document::new(
        "Design doc".to_string(),
        "initial body".to_string(),
        Some("docs/design.md".to_string()),
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
            Document::new("Doc v1".to_string(), "body v1".to_string(), None, false).unwrap(),
        ))
        .send()
        .await?
        .json()
        .await?;

    let _updated: UpsertDocumentResponse = client
        .put(format!("{base}/v1/documents/{}", created.document_id))
        .json(&UpsertDocumentRequest::new(
            Document::new("Doc v2".to_string(), "body v2".to_string(), None, false).unwrap(),
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

/// Helpers shared by the actor-based running-job tests.
fn client_with_token(token: &str) -> Client {
    let mut headers = header::HeaderMap::new();
    let auth_value = format!("Bearer {token}");
    headers.insert(
        header::AUTHORIZATION,
        header::HeaderValue::from_str(&auth_value).expect("valid auth header"),
    );
    Client::builder()
        .default_headers(headers)
        .build()
        .expect("failed to build client")
}

fn empty_doc() -> Document {
    Document::new("Doc".to_string(), "body".to_string(), None, false).unwrap()
}

/// Once a session is killed, every auth token minted by that session is
/// flagged `is_revoked = TRUE` and `require_auth` rejects them with 401.
///
/// This regression test pins that contract end-to-end. We seed an actor
/// + an `auth_tokens` row keyed to a session id, then call the store-side
/// `revoke_auth_tokens_for_session` (the same call `routes/sessions/kill.rs`
/// makes after the job engine acknowledges the kill). The subsequent
/// request from the revoked token must fail at the auth layer (401), not
/// at any downstream application policy (403/400).
#[tokio::test]
async fn killed_session_token_is_rejected_at_auth() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let creator = Username::from("test-creator");
    let session_id = SessionId::new();
    let (actor, auth_token) =
        Actor::new_from_actor_id(ActorId::Adhoc(session_id.clone()), creator, None);
    // Issue an auth-token row bound to the session — same shape that
    // `create_actor_for_job` writes in production.
    crate::test_utils::register_actor_and_token(
        handles.store.as_ref(),
        &actor,
        &auth_token,
        Some(&session_id),
    )
    .await?;

    let store = handles.store.clone();
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = client_with_token(&auth_token);

    // Sanity check: before revocation the token authenticates.
    let response = client
        .post(format!("{}/v1/documents", server.base_url()))
        .json(&UpsertDocumentRequest::new(empty_doc()))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    // Simulate `sessions/kill` revoking the token.
    store.revoke_auth_tokens_for_session(&session_id).await?;

    let response = client
        .post(format!("{}/v1/documents", server.base_url()))
        .json(&UpsertDocumentRequest::new(empty_doc()))
        .send()
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "revoked token must be rejected at the auth layer, not by a downstream policy"
    );

    Ok(())
}

#[tokio::test]
async fn documents_support_search_filters() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let task = sample_task(Status::Running);
    let (running_task, _) = handles
        .store
        .add_session(task.clone(), Utc::now(), &ActorRef::test())
        .await?;
    handles
        .store
        .update_session(&running_task, task, &ActorRef::test())
        .await?;
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();
    let base = server.base_url();

    let docs = [
        Document::new(
            "Runbook".to_string(),
            "operations".to_string(),
            Some("docs/runbook.md".to_string()),
            false,
        )
        .unwrap(),
        Document::new(
            "API Guide".to_string(),
            "api details".to_string(),
            Some("docs/guide.md".to_string()),
            false,
        )
        .unwrap(),
        Document::new(
            "Notes".to_string(),
            "private".to_string(),
            Some("notes/internal.md".to_string()),
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
            Some("/docs/".to_string()),
            None,
            None,
        ))
        .send()
        .await?
        .json::<ListDocumentsResponse>()
        .await?;
    assert_eq!(by_path.documents.len(), 2);

    Ok(())
}

#[tokio::test]
async fn documents_filter_by_has_path() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    // Create a document with a path
    let with_path = Document::new(
        "With Path".to_string(),
        "has a path".to_string(),
        Some("docs/file.md".to_string()),
        false,
    )
    .unwrap();

    // Create a document without a path
    let without_path = Document::new(
        "Without Path".to_string(),
        "no path".to_string(),
        None,
        false,
    )
    .unwrap();

    for doc in [&with_path, &without_path] {
        client
            .post(format!("{base}/v1/documents"))
            .json(&UpsertDocumentRequest::new(doc.clone()))
            .send()
            .await?
            .error_for_status()?;
    }

    // has_path=true returns only documents with a path. The
    // `spawn_test_server` helper seeds a `/agents/.../prompt.md` document
    // for the default conversation agent, so filter that one out before
    // checking the test-created documents.
    let mut query = SearchDocumentsQuery::default();
    query.has_path = Some(true);
    let result: ListDocumentsResponse = client
        .get(format!("{base}/v1/documents"))
        .query(&query)
        .send()
        .await?
        .json()
        .await?;
    let with_path_test_docs: Vec<_> = result
        .documents
        .iter()
        .filter(|d| d.document.title != "default test agent prompt")
        .collect();
    assert_eq!(with_path_test_docs.len(), 1);
    assert_eq!(with_path_test_docs[0].document.title, "With Path");

    // has_path=false returns only documents without a path
    let mut query = SearchDocumentsQuery::default();
    query.has_path = Some(false);
    let result: ListDocumentsResponse = client
        .get(format!("{base}/v1/documents"))
        .query(&query)
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(result.documents.len(), 1);
    assert_eq!(result.documents[0].document.title, "Without Path");

    // No has_path filter returns all test-created documents (plus the seeded
    // default agent prompt).
    let result: ListDocumentsResponse = client
        .get(format!("{base}/v1/documents"))
        .send()
        .await?
        .json()
        .await?;
    let test_docs: Vec<_> = result
        .documents
        .iter()
        .filter(|d| d.document.title != "default test agent prompt")
        .collect();
    assert_eq!(test_docs.len(), 2);

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
            false,
        )
        .unwrap(),
        Document::new(
            "Prefix Doc".to_string(),
            "prefix match".to_string(),
            Some("docs/guide.md.bak".to_string()),
            false,
        )
        .unwrap(),
        Document::new(
            "Nested Doc".to_string(),
            "nested match".to_string(),
            Some("docs/guide.md/extra".to_string()),
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
    let archived: DocumentVersionRecord = client
        .delete(format!("{base}/v1/documents/{}", created.document_id))
        .send()
        .await?
        .json()
        .await?;

    // Verify the response has archived=true
    assert!(archived.document.archived);

    // Verify listing excludes the archived document
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

    // List without include_archived - verify not present
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

    // List with include_archived=true - verify present with archived=true
    let list_with: ListDocumentsResponse = client
        .get(format!("{base}/v1/documents"))
        .query(&SearchDocumentsQuery::new(None, None, None, Some(true)))
        .send()
        .await?
        .json()
        .await?;

    let deleted_doc = list_with
        .documents
        .iter()
        .find(|d| d.document_id == created.document_id);

    assert!(deleted_doc.is_some());
    assert!(deleted_doc.unwrap().document.archived);

    Ok(())
}

#[tokio::test]
async fn delete_document_get_deleted_by_id() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    // Create and delete a document
    let document = Document::new(
        "Get archived doc".to_string(),
        "document body".to_string(),
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

    // GET by ID should return 404 for archived documents
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
            Document::new("Doc v1".to_string(), "body v1".to_string(), None, false).unwrap(),
        ))
        .send()
        .await?
        .json()
        .await?;

    // Update document (v2)
    client
        .put(format!("{base}/v1/documents/{}", created.document_id))
        .json(&UpsertDocumentRequest::new(
            Document::new("Doc v2".to_string(), "body v2".to_string(), None, false).unwrap(),
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

// ===== /v1/documents/paths tests =====

async fn create_doc_at(
    client: &reqwest::Client,
    base: &str,
    title: &str,
    path: &str,
) -> anyhow::Result<DocumentId> {
    let doc = Document::new(
        title.to_string(),
        "body".to_string(),
        Some(path.to_string()),
        false,
    )
    .unwrap();
    let created: UpsertDocumentResponse = client
        .post(format!("{base}/v1/documents"))
        .json(&UpsertDocumentRequest::new(doc))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(created.document_id)
}

#[tokio::test]
async fn list_document_paths_single_prefix_inlines_document_ref() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    create_doc_at(&client, &base, "SWE Memory", "agents/swe/memory.md").await?;
    create_doc_at(&client, &base, "SWE Plan", "agents/swe/plan.md").await?;
    let pm_id = create_doc_at(&client, &base, "PM Notes", "agents/pm").await?;

    let response: ListDocumentPathsResponse = client
        .get(format!("{base}/v1/documents/paths?prefix=/agents/"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let mut by_name: HashMap<&str, &hydra_common::api::v1::documents::PathChildEntry> =
        HashMap::new();
    for entry in &response.children {
        by_name.insert(entry.name.as_str(), entry);
    }

    let swe = by_name.get("swe").expect("/agents/swe entry");
    assert_eq!(swe.full_path, "/agents/swe");
    assert!(!swe.is_document);
    assert!(
        swe.document.is_none(),
        "folder entries must not have inline document ref"
    );

    let pm = by_name.get("pm").expect("/agents/pm entry");
    assert_eq!(pm.full_path, "/agents/pm");
    assert!(pm.is_document);
    let doc_ref = pm.document.as_ref().expect("inline document ref expected");
    assert_eq!(doc_ref.document_id, pm_id);
    assert_eq!(doc_ref.title, "PM Notes");

    Ok(())
}

#[tokio::test]
async fn list_document_paths_multi_prefix_unions_and_inlines_refs() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    let agents_pm = create_doc_at(&client, &base, "PM Notes", "agents/pm").await?;
    create_doc_at(&client, &base, "SWE Memory", "agents/swe/memory.md").await?;
    let repos_readme = create_doc_at(&client, &base, "Readme", "repos/hydra").await?;

    let response: ListDocumentPathsResponse = client
        .get(format!(
            "{base}/v1/documents/paths?prefixes=/agents/,/repos/"
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    // /agents/ entries must precede /repos/ entries in the union (prefixes
    // processed in input order).
    let positions: HashMap<&str, usize> = response
        .children
        .iter()
        .enumerate()
        .map(|(i, e)| (e.full_path.as_str(), i))
        .collect();
    let pm_idx = positions["/agents/pm"];
    let swe_idx = positions["/agents/swe"];
    let hydra_idx = positions["/repos/hydra"];
    assert!(pm_idx < hydra_idx);
    assert!(swe_idx < hydra_idx);

    let pm = response
        .children
        .iter()
        .find(|e| e.full_path == "/agents/pm")
        .unwrap();
    let pm_ref = pm
        .document
        .as_ref()
        .expect("/agents/pm should have doc ref");
    assert_eq!(pm_ref.document_id, agents_pm);
    assert_eq!(pm_ref.title, "PM Notes");

    let hydra = response
        .children
        .iter()
        .find(|e| e.full_path == "/repos/hydra")
        .unwrap();
    let hydra_ref = hydra
        .document
        .as_ref()
        .expect("/repos/hydra should have doc ref");
    assert_eq!(hydra_ref.document_id, repos_readme);
    assert_eq!(hydra_ref.title, "Readme");

    Ok(())
}

#[tokio::test]
async fn list_document_paths_rejects_prefix_and_prefixes_together() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let response = client
        .get(format!(
            "{}/v1/documents/paths?prefix=/agents/&prefixes=/repos/",
            server.base_url()
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn list_document_paths_empty_query_returns_root_listing() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    create_doc_at(&client, &base, "Readme", "repos/hydra/README.md").await?;

    let response: ListDocumentPathsResponse = client
        .get(format!("{base}/v1/documents/paths"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    // `spawn_test_server` seeds /agents/default-test-agent/prompt.md, so root
    // listing always contains at least /agents.
    let segments: std::collections::HashSet<&str> =
        response.children.iter().map(|e| e.name.as_str()).collect();
    assert!(segments.contains("agents"));
    assert!(segments.contains("repos"));

    // Top-level folders should never carry a document ref because they are not
    // themselves direct documents in this fixture.
    for entry in &response.children {
        if !entry.is_document {
            assert!(entry.document.is_none());
        }
    }
    Ok(())
}

#[tokio::test]
async fn list_document_paths_omits_document_ref_when_doc_is_deleted() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    // /agents/swe is itself a document AND a folder for /agents/swe/memory.md.
    let swe_doc = create_doc_at(&client, &base, "SWE Root", "agents/swe").await?;
    create_doc_at(&client, &base, "SWE Memory", "agents/swe/memory.md").await?;

    // Sanity: before deletion the entry has both is_document=true and an
    // inline document ref.
    let before: ListDocumentPathsResponse = client
        .get(format!("{base}/v1/documents/paths?prefix=/agents/"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let swe_before = before
        .children
        .iter()
        .find(|e| e.full_path == "/agents/swe")
        .expect("/agents/swe entry");
    assert!(swe_before.is_document);
    let swe_ref = swe_before.document.as_ref().unwrap();
    assert_eq!(swe_ref.document_id, swe_doc);

    // Delete the /agents/swe document. The folder still has a child
    // (/agents/swe/memory.md), so the entry should become is_document=false
    // with no inline document ref.
    client
        .delete(format!("{base}/v1/documents/{swe_doc}"))
        .send()
        .await?
        .error_for_status()?;

    let after: ListDocumentPathsResponse = client
        .get(format!("{base}/v1/documents/paths?prefix=/agents/"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let swe_after = after
        .children
        .iter()
        .find(|e| e.full_path == "/agents/swe")
        .expect("/agents/swe entry still exists due to children");
    assert!(!swe_after.is_document);
    assert!(swe_after.document.is_none());

    Ok(())
}
