use super::common::{patch_diff, service_repo_name};
use crate::{
    domain::{
        issues::{Issue, IssueStatus, IssueType},
        patches::{Patch, PatchStatus},
        users::Username,
    },
    test_utils::{spawn_test_server, test_client},
};
use hydra_common::api::v1::{
    documents::{
        Document, DocumentVersionRecord, ListDocumentVersionsResponse, ListDocumentsResponse,
        UpsertDocumentRequest, UpsertDocumentResponse,
    },
    issues::{
        IssueVersionRecord, ListIssueVersionsResponse, ListIssuesResponse, UpsertIssueRequest,
        UpsertIssueResponse,
    },
    labels::{Label, UpsertLabelRequest, UpsertLabelResponse},
    patches::{
        ListPatchVersionsResponse, ListPatchesResponse, PatchVersionRecord, UpsertPatchRequest,
        UpsertPatchResponse,
    },
};

fn default_user() -> Username {
    Username::from("creator")
}

/// Helper: create a label and associate it with an object, returning the label name.
async fn create_and_assign_label(
    client: &reqwest::Client,
    base_url: &str,
    object_id: &str,
) -> anyhow::Result<String> {
    let label_name = format!(
        "test-label-{}",
        object_id.chars().take(8).collect::<String>()
    );
    let label_resp: UpsertLabelResponse = client
        .post(format!("{base_url}/v1/labels"))
        .json(&UpsertLabelRequest::new(Label::new(
            label_name.clone(),
            None,
        )))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    client
        .put(format!(
            "{base_url}/v1/labels/{}/objects/{object_id}",
            label_resp.label_id
        ))
        .send()
        .await?
        .error_for_status()?;

    Ok(label_name)
}

// ===== Issue label tests =====

#[tokio::test]
async fn issue_labels_returned_from_all_routes() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    // Create an issue
    let created: UpsertIssueResponse = client
        .post(format!("{base}/v1/issues"))
        .json(&UpsertIssueRequest::new(
            Issue::new(
                IssueType::Task,
                "Test Title".to_string(),
                "label test issue".to_string(),
                default_user(),
                String::new(),
                IssueStatus::Open,
                None,
                None,
                Vec::new(),
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    let issue_id = &created.issue_id;
    let label_name = create_and_assign_label(&client, &base, issue_id.as_ref()).await?;

    // get_issue
    let fetched: IssueVersionRecord = client
        .get(format!("{base}/v1/issues/{issue_id}"))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(fetched.labels.len(), 1, "get_issue should return labels");
    assert_eq!(fetched.labels[0].name, label_name);

    // list_issues
    let list: ListIssuesResponse = client
        .get(format!("{base}/v1/issues"))
        .send()
        .await?
        .json()
        .await?;
    let found = list
        .issues
        .iter()
        .find(|i| &i.issue_id == issue_id)
        .unwrap();
    assert_eq!(
        found.issue.labels.len(),
        1,
        "list_issues should return labels"
    );
    assert_eq!(found.issue.labels[0].name, label_name);

    // list_issue_versions
    let versions: ListIssueVersionsResponse = client
        .get(format!("{base}/v1/issues/{issue_id}/versions"))
        .send()
        .await?
        .json()
        .await?;
    assert!(!versions.versions.is_empty());
    for v in &versions.versions {
        assert_eq!(
            v.labels.len(),
            1,
            "list_issue_versions should return labels on each version"
        );
        assert_eq!(v.labels[0].name, label_name);
    }

    // get_issue_version
    let version: IssueVersionRecord = client
        .get(format!("{base}/v1/issues/{issue_id}/versions/1"))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(
        version.labels.len(),
        1,
        "get_issue_version should return labels"
    );
    assert_eq!(version.labels[0].name, label_name);

    // delete_issue
    let deleted: IssueVersionRecord = client
        .delete(format!("{base}/v1/issues/{issue_id}"))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(deleted.labels.len(), 1, "delete_issue should return labels");
    assert_eq!(deleted.labels[0].name, label_name);

    Ok(())
}

// ===== Patch label tests =====

#[tokio::test]
async fn patch_labels_returned_from_all_routes() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    let patch = Patch::new(
        "Label test patch".to_string(),
        "label test patch".to_string(),
        patch_diff(),
        PatchStatus::Open,
        false,
        None,
        Username::from("test-creator"),
        Vec::new(),
        service_repo_name(),
        None,
        None,
        None,
        None,
    );

    let created: UpsertPatchResponse = client
        .post(format!("{base}/v1/patches"))
        .json(&UpsertPatchRequest::new(patch.into()))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let patch_id = &created.patch_id;
    let label_name = create_and_assign_label(&client, &base, patch_id.as_ref()).await?;

    // get_patch
    let fetched: PatchVersionRecord = client
        .get(format!("{base}/v1/patches/{patch_id}"))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(fetched.labels.len(), 1, "get_patch should return labels");
    assert_eq!(fetched.labels[0].name, label_name);

    // list_patches
    let list: ListPatchesResponse = client
        .get(format!("{base}/v1/patches"))
        .send()
        .await?
        .json()
        .await?;
    let found = list
        .patches
        .iter()
        .find(|p| &p.patch_id == patch_id)
        .unwrap();
    assert_eq!(
        found.patch.labels.len(),
        1,
        "list_patches should return labels"
    );
    assert_eq!(found.patch.labels[0].name, label_name);

    // list_patch_versions
    let versions: ListPatchVersionsResponse = client
        .get(format!("{base}/v1/patches/{patch_id}/versions"))
        .send()
        .await?
        .json()
        .await?;
    assert!(!versions.versions.is_empty());
    for v in &versions.versions {
        assert_eq!(
            v.labels.len(),
            1,
            "list_patch_versions should return labels on each version"
        );
        assert_eq!(v.labels[0].name, label_name);
    }

    // get_patch_version
    let version: PatchVersionRecord = client
        .get(format!("{base}/v1/patches/{patch_id}/versions/1"))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(
        version.labels.len(),
        1,
        "get_patch_version should return labels"
    );
    assert_eq!(version.labels[0].name, label_name);

    // delete_patch
    let deleted: PatchVersionRecord = client
        .delete(format!("{base}/v1/patches/{patch_id}"))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(deleted.labels.len(), 1, "delete_patch should return labels");
    assert_eq!(deleted.labels[0].name, label_name);

    Ok(())
}

// ===== Document label tests =====

#[tokio::test]
async fn document_labels_returned_from_all_routes() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    let created: UpsertDocumentResponse = client
        .post(format!("{base}/v1/documents"))
        .json(&UpsertDocumentRequest::new(
            Document::new(
                "Label test doc".to_string(),
                "label test body".to_string(),
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

    let doc_id = &created.document_id;
    let label_name = create_and_assign_label(&client, &base, doc_id.as_ref()).await?;

    // get_document
    let fetched: DocumentVersionRecord = client
        .get(format!("{base}/v1/documents/{doc_id}"))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(fetched.labels.len(), 1, "get_document should return labels");
    assert_eq!(fetched.labels[0].name, label_name);

    // list_documents
    let list: ListDocumentsResponse = client
        .get(format!("{base}/v1/documents"))
        .send()
        .await?
        .json()
        .await?;
    let found = list
        .documents
        .iter()
        .find(|d| &d.document_id == doc_id)
        .unwrap();
    assert_eq!(
        found.document.labels.len(),
        1,
        "list_documents should return labels"
    );
    assert_eq!(found.document.labels[0].name, label_name);

    // list_document_versions
    let versions: ListDocumentVersionsResponse = client
        .get(format!("{base}/v1/documents/{doc_id}/versions"))
        .send()
        .await?
        .json()
        .await?;
    assert!(!versions.versions.is_empty());
    for v in &versions.versions {
        assert_eq!(
            v.labels.len(),
            1,
            "list_document_versions should return labels on each version"
        );
        assert_eq!(v.labels[0].name, label_name);
    }

    // get_document_version
    let version: DocumentVersionRecord = client
        .get(format!("{base}/v1/documents/{doc_id}/versions/1"))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(
        version.labels.len(),
        1,
        "get_document_version should return labels"
    );
    assert_eq!(version.labels[0].name, label_name);

    // delete_document
    let deleted: DocumentVersionRecord = client
        .delete(format!("{base}/v1/documents/{doc_id}"))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(
        deleted.labels.len(),
        1,
        "delete_document should return labels"
    );
    assert_eq!(deleted.labels[0].name, label_name);

    Ok(())
}
