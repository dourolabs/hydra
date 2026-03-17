use crate::domain::issues::{Issue, IssueStatus, IssueType};
use crate::domain::users::Username;
use crate::test_utils::{spawn_test_server, test_client};
use metis_common::MetisId;
use metis_common::api::v1::{
    issues::{UpsertIssueRequest, UpsertIssueResponse},
    relations::{
        CreateRelationRequest, ListRelationsResponse, RelationResponse, RemoveRelationRequest,
        RemoveRelationResponse,
    },
};

fn default_user() -> Username {
    Username::from("creator")
}

/// Helper: create an issue and return its ID.
async fn create_issue(
    client: &reqwest::Client,
    base: &str,
    title: &str,
) -> anyhow::Result<MetisId> {
    let resp: UpsertIssueResponse = client
        .post(format!("{base}/v1/issues"))
        .json(&UpsertIssueRequest::new(
            Issue::new(
                IssueType::Task,
                title.to_string(),
                format!("description for {title}"),
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
        .error_for_status()?
        .json()
        .await?;
    Ok(MetisId::from(resp.issue_id))
}

// ===== POST /v1/relations =====

#[tokio::test]
async fn create_relation_returns_201() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    let source_id = create_issue(&client, &base, "source issue").await?;
    let target_id = create_issue(&client, &base, "target issue").await?;

    let resp = client
        .post(format!("{base}/v1/relations"))
        .json(&CreateRelationRequest {
            source_id: source_id.clone(),
            target_id: target_id.clone(),
            rel_type: "child-of".to_string(),
        })
        .send()
        .await?;

    assert_eq!(resp.status(), 201, "new relation should return 201");

    let body: RelationResponse = resp.json().await?;
    assert_eq!(body.source_id, source_id);
    assert_eq!(body.target_id, target_id);
    assert_eq!(body.rel_type, "child-of");

    Ok(())
}

#[tokio::test]
async fn create_duplicate_relation_returns_200() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    let source_id = create_issue(&client, &base, "source").await?;
    let target_id = create_issue(&client, &base, "target").await?;

    let req = CreateRelationRequest {
        source_id: source_id.clone(),
        target_id: target_id.clone(),
        rel_type: "child-of".to_string(),
    };

    // First creation
    let resp1 = client
        .post(format!("{base}/v1/relations"))
        .json(&req)
        .send()
        .await?;
    assert_eq!(resp1.status(), 201);

    // Duplicate creation
    let resp2 = client
        .post(format!("{base}/v1/relations"))
        .json(&req)
        .send()
        .await?;
    assert_eq!(resp2.status(), 200, "duplicate should return 200");

    Ok(())
}

// ===== GET /v1/relations =====

#[tokio::test]
async fn list_relations_by_source_id() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    let source_id = create_issue(&client, &base, "source").await?;
    let target_id = create_issue(&client, &base, "target").await?;

    client
        .post(format!("{base}/v1/relations"))
        .json(&CreateRelationRequest {
            source_id: source_id.clone(),
            target_id: target_id.clone(),
            rel_type: "child-of".to_string(),
        })
        .send()
        .await?
        .error_for_status()?;

    let list: ListRelationsResponse = client
        .get(format!("{base}/v1/relations?source_id={source_id}"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    assert_eq!(list.relations.len(), 1);
    assert_eq!(list.relations[0].source_id, source_id);
    assert_eq!(list.relations[0].target_id, target_id);

    Ok(())
}

#[tokio::test]
async fn list_relations_by_object_id() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    let a = create_issue(&client, &base, "issue-a").await?;
    let b = create_issue(&client, &base, "issue-b").await?;
    let c = create_issue(&client, &base, "issue-c").await?;

    // a -> b (child-of)
    client
        .post(format!("{base}/v1/relations"))
        .json(&CreateRelationRequest {
            source_id: a.clone(),
            target_id: b.clone(),
            rel_type: "child-of".to_string(),
        })
        .send()
        .await?
        .error_for_status()?;

    // c -> a (blocked-on)
    client
        .post(format!("{base}/v1/relations"))
        .json(&CreateRelationRequest {
            source_id: c.clone(),
            target_id: a.clone(),
            rel_type: "blocked-on".to_string(),
        })
        .send()
        .await?
        .error_for_status()?;

    // Query object_id=a should return both relations
    let list: ListRelationsResponse = client
        .get(format!("{base}/v1/relations?object_id={a}"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    assert_eq!(
        list.relations.len(),
        2,
        "object_id should find relations where object is source or target"
    );

    Ok(())
}

#[tokio::test]
async fn list_relations_validation_rejects_mutually_exclusive_params() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    // source_id + source_ids
    let resp = client
        .get(format!("{base}/v1/relations?source_id=x&source_ids=x,y"))
        .send()
        .await?;
    assert_eq!(resp.status(), 400);

    // target_id + target_ids
    let resp = client
        .get(format!("{base}/v1/relations?target_id=x&target_ids=x,y"))
        .send()
        .await?;
    assert_eq!(resp.status(), 400);

    // object_id + source_id
    let resp = client
        .get(format!("{base}/v1/relations?object_id=x&source_id=y"))
        .send()
        .await?;
    assert_eq!(resp.status(), 400);

    Ok(())
}

#[tokio::test]
async fn list_relations_transitive_validation() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    // transitive without rel_type
    let resp = client
        .get(format!("{base}/v1/relations?source_id=x&transitive=true"))
        .send()
        .await?;
    assert_eq!(resp.status(), 400);

    // transitive without source_id or target_id
    let resp = client
        .get(format!(
            "{base}/v1/relations?transitive=true&rel_type=child-of"
        ))
        .send()
        .await?;
    assert_eq!(resp.status(), 400);

    Ok(())
}

#[tokio::test]
async fn list_relations_batch_cap_at_100() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    // Build a source_ids string with 101 IDs
    let ids: Vec<String> = (0..101).map(|i| format!("id-{i}")).collect();
    let ids_str = ids.join(",");

    let resp = client
        .get(format!("{base}/v1/relations?source_ids={ids_str}"))
        .send()
        .await?;
    assert_eq!(
        resp.status(),
        400,
        "batch query with >100 IDs should return 400"
    );

    Ok(())
}

// ===== DELETE /v1/relations =====

#[tokio::test]
async fn remove_relation_returns_removed_true() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    let source_id = create_issue(&client, &base, "source").await?;
    let target_id = create_issue(&client, &base, "target").await?;

    // Create relation
    client
        .post(format!("{base}/v1/relations"))
        .json(&CreateRelationRequest {
            source_id: source_id.clone(),
            target_id: target_id.clone(),
            rel_type: "child-of".to_string(),
        })
        .send()
        .await?
        .error_for_status()?;

    // Remove it
    let resp: RemoveRelationResponse = client
        .delete(format!("{base}/v1/relations"))
        .json(&RemoveRelationRequest {
            source_id: source_id.clone(),
            target_id: target_id.clone(),
            rel_type: "child-of".to_string(),
        })
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    assert!(
        resp.removed,
        "removing existing relation should return true"
    );

    // Verify it's gone
    let list: ListRelationsResponse = client
        .get(format!("{base}/v1/relations?source_id={source_id}"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    assert!(
        list.relations.is_empty(),
        "relation should be gone after removal"
    );

    Ok(())
}

#[tokio::test]
async fn remove_nonexistent_relation_returns_removed_false() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    let source_id = create_issue(&client, &base, "source").await?;
    let target_id = create_issue(&client, &base, "target").await?;

    let resp: RemoveRelationResponse = client
        .delete(format!("{base}/v1/relations"))
        .json(&RemoveRelationRequest {
            source_id,
            target_id,
            rel_type: "child-of".to_string(),
        })
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    assert!(
        !resp.removed,
        "removing nonexistent relation should return false"
    );

    Ok(())
}

// ===== Batch transitive queries =====

#[tokio::test]
async fn list_relations_batch_transitive_with_target_ids() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    // Create a tree: a -> b -> c, d -> b (both child-of)
    let a = create_issue(&client, &base, "issue-a").await?;
    let b = create_issue(&client, &base, "issue-b").await?;
    let c = create_issue(&client, &base, "issue-c").await?;
    let d = create_issue(&client, &base, "issue-d").await?;

    for (src, tgt) in [(&a, &b), (&b, &c), (&d, &b)] {
        client
            .post(format!("{base}/v1/relations"))
            .json(&CreateRelationRequest {
                source_id: src.clone(),
                target_id: tgt.clone(),
                rel_type: "child-of".to_string(),
            })
            .send()
            .await?
            .error_for_status()?;
    }

    // Batch transitive query with target_ids=b,c should return all ancestors of b and c
    let list: ListRelationsResponse = client
        .get(format!(
            "{base}/v1/relations?target_ids={b},{c}&rel_type=child-of&transitive=true"
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    // b has children a and d; c has child b which has children a and d
    // target_ids traversal goes backward: for target b, finds a->b and d->b;
    // for target c, finds b->c, then a->b, d->b
    assert!(
        list.relations.len() >= 3,
        "batch transitive should return union of transitive results, got {}",
        list.relations.len()
    );

    Ok(())
}

#[tokio::test]
async fn list_relations_batch_transitive_with_source_ids() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    // a -> b -> c, d -> e (both child-of)
    let a = create_issue(&client, &base, "issue-a").await?;
    let b = create_issue(&client, &base, "issue-b").await?;
    let c = create_issue(&client, &base, "issue-c").await?;
    let d = create_issue(&client, &base, "issue-d").await?;
    let e = create_issue(&client, &base, "issue-e").await?;

    for (src, tgt) in [(&a, &b), (&b, &c), (&d, &e)] {
        client
            .post(format!("{base}/v1/relations"))
            .json(&CreateRelationRequest {
                source_id: src.clone(),
                target_id: tgt.clone(),
                rel_type: "child-of".to_string(),
            })
            .send()
            .await?
            .error_for_status()?;
    }

    // Batch transitive from source_ids=a,d
    let list: ListRelationsResponse = client
        .get(format!(
            "{base}/v1/relations?source_ids={a},{d}&rel_type=child-of&transitive=true"
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    // a->b, b->c, d->e = 3 relations
    assert_eq!(
        list.relations.len(),
        3,
        "batch transitive from two source_ids should return union"
    );

    Ok(())
}

#[tokio::test]
async fn list_relations_batch_transitive_single_id_in_plural_param() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    let a = create_issue(&client, &base, "issue-a").await?;
    let b = create_issue(&client, &base, "issue-b").await?;

    client
        .post(format!("{base}/v1/relations"))
        .json(&CreateRelationRequest {
            source_id: a.clone(),
            target_id: b.clone(),
            rel_type: "child-of".to_string(),
        })
        .send()
        .await?
        .error_for_status()?;

    // Single ID in plural param should work like singular
    let list: ListRelationsResponse = client
        .get(format!(
            "{base}/v1/relations?source_ids={a}&rel_type=child-of&transitive=true"
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    assert_eq!(list.relations.len(), 1);
    assert_eq!(list.relations[0].source_id, a);
    assert_eq!(list.relations[0].target_id, b);

    Ok(())
}

#[tokio::test]
async fn list_relations_batch_transitive_empty_results() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    let a = create_issue(&client, &base, "issue-a").await?;

    // No relations exist, transitive should return empty
    let list: ListRelationsResponse = client
        .get(format!(
            "{base}/v1/relations?source_ids={a}&rel_type=child-of&transitive=true"
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    assert!(
        list.relations.is_empty(),
        "transitive with no matching relations should return empty"
    );

    Ok(())
}

#[tokio::test]
async fn list_relations_batch_transitive_validation_rejects_both_directions() -> anyhow::Result<()>
{
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    // Both source_ids and target_ids with transitive should fail
    let resp = client
        .get(format!(
            "{base}/v1/relations?source_ids=x&target_ids=y&rel_type=child-of&transitive=true"
        ))
        .send()
        .await?;
    assert_eq!(resp.status(), 400);

    Ok(())
}

#[tokio::test]
async fn list_relations_singular_transitive_still_works() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    let a = create_issue(&client, &base, "issue-a").await?;
    let b = create_issue(&client, &base, "issue-b").await?;
    let c = create_issue(&client, &base, "issue-c").await?;

    for (src, tgt) in [(&a, &b), (&b, &c)] {
        client
            .post(format!("{base}/v1/relations"))
            .json(&CreateRelationRequest {
                source_id: src.clone(),
                target_id: tgt.clone(),
                rel_type: "child-of".to_string(),
            })
            .send()
            .await?
            .error_for_status()?;
    }

    // Existing singular source_id + transitive should still work
    let list: ListRelationsResponse = client
        .get(format!(
            "{base}/v1/relations?source_id={a}&rel_type=child-of&transitive=true"
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    assert_eq!(list.relations.len(), 2, "a->b and b->c");

    // Existing singular target_id + transitive should still work
    let list: ListRelationsResponse = client
        .get(format!(
            "{base}/v1/relations?target_id={c}&rel_type=child-of&transitive=true"
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    assert_eq!(list.relations.len(), 2, "b->c and a->b");

    Ok(())
}
