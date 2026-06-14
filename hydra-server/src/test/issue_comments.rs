//! HTTP route tests for per-issue comments
//! (`/v1/issues/:issue_id/comments`).

use crate::{
    domain::{
        actors::Actor,
        issues::{Issue, IssueType},
        users::Username,
    },
    test_utils::{
        add_agent_with_name, register_actor_and_token, spawn_test_server,
        spawn_test_server_with_state, test_client, test_state_handles,
    },
};
use hydra_common::{
    ActorId, ActorRef, IssueId,
    api::v1::{
        agents::AgentName,
        comments::{AddCommentRequest, AddCommentResponse, ListCommentsResponse},
        issues::{UpsertIssueRequest, UpsertIssueResponse},
    },
    test_utils::status::status,
};
use reqwest::{Client, StatusCode, header};

fn default_user() -> Username {
    Username::from("creator")
}

fn make_issue() -> Issue {
    Issue::new(
        IssueType::Task,
        "comments target".to_string(),
        "issue used to hang comments off of".to_string(),
        default_user(),
        status("open"),
        crate::domain::projects::default_project_id(),
        None,
        None,
        Vec::new(),
        Vec::new(),
        None,
        None,
    )
}

async fn create_issue(client: &Client, base: &str) -> anyhow::Result<IssueId> {
    let created: UpsertIssueResponse = client
        .post(format!("{base}/v1/issues"))
        .json(&UpsertIssueRequest::new(make_issue().into(), None))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(created.issue_id)
}

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

#[tokio::test]
async fn post_then_get_round_trip() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();
    let issue_id = create_issue(&client, &base).await?;

    let added: AddCommentResponse = client
        .post(format!("{base}/v1/issues/{issue_id}/comments"))
        .json(&AddCommentRequest::new("hello world".to_string()))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    assert_eq!(added.comment.issue_id, issue_id);
    assert_eq!(added.comment.sequence, 1);
    assert_eq!(added.comment.body, "hello world");

    let listed: ListCommentsResponse = client
        .get(format!("{base}/v1/issues/{issue_id}/comments"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(listed.comments.len(), 1);
    assert_eq!(listed.comments[0].sequence, 1);
    assert_eq!(listed.comments[0].body, "hello world");
    assert!(listed.next_before_sequence.is_none());

    Ok(())
}

#[tokio::test]
async fn sequence_increments_per_issue() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();
    let issue_id = create_issue(&client, &base).await?;

    for (idx, body) in ["first", "second", "third"].iter().enumerate() {
        let resp: AddCommentResponse = client
            .post(format!("{base}/v1/issues/{issue_id}/comments"))
            .json(&AddCommentRequest::new(body.to_string()))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        assert_eq!(resp.comment.sequence, (idx + 1) as u64);
    }

    let listed: ListCommentsResponse = client
        .get(format!("{base}/v1/issues/{issue_id}/comments"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let sequences: Vec<u64> = listed.comments.iter().map(|c| c.sequence).collect();
    assert_eq!(sequences, vec![3, 2, 1]);

    Ok(())
}

#[tokio::test]
async fn pagination_walks_pages_in_desc_order() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();
    let issue_id = create_issue(&client, &base).await?;

    for i in 1..=25u64 {
        client
            .post(format!("{base}/v1/issues/{issue_id}/comments"))
            .json(&AddCommentRequest::new(format!("comment {i}")))
            .send()
            .await?
            .error_for_status()?;
    }

    let page1: ListCommentsResponse = client
        .get(format!("{base}/v1/issues/{issue_id}/comments"))
        .query(&[("limit", "10")])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(page1.comments.len(), 10);
    let seqs1: Vec<u64> = page1.comments.iter().map(|c| c.sequence).collect();
    assert_eq!(seqs1, (16..=25).rev().collect::<Vec<_>>());
    assert_eq!(page1.next_before_sequence, Some(16));

    let page2: ListCommentsResponse = client
        .get(format!("{base}/v1/issues/{issue_id}/comments"))
        .query(&[("limit", "10"), ("before_sequence", "16")])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(page2.comments.len(), 10);
    let seqs2: Vec<u64> = page2.comments.iter().map(|c| c.sequence).collect();
    assert_eq!(seqs2, (6..=15).rev().collect::<Vec<_>>());
    assert_eq!(page2.next_before_sequence, Some(6));

    let page3: ListCommentsResponse = client
        .get(format!("{base}/v1/issues/{issue_id}/comments"))
        .query(&[("limit", "10"), ("before_sequence", "6")])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(page3.comments.len(), 5);
    let seqs3: Vec<u64> = page3.comments.iter().map(|c| c.sequence).collect();
    assert_eq!(seqs3, (1..=5).rev().collect::<Vec<_>>());
    assert!(page3.next_before_sequence.is_none());

    Ok(())
}

#[tokio::test]
async fn post_on_unknown_issue_returns_404() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();
    let missing: IssueId = "i-aaaaaaaa".parse().expect("static id");

    let resp = client
        .post(format!("{base}/v1/issues/{missing}/comments"))
        .json(&AddCommentRequest::new("hi".to_string()))
        .send()
        .await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn get_on_unknown_issue_returns_404() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();
    let missing: IssueId = "i-aaaaaaaa".parse().expect("static id");

    let resp = client
        .get(format!("{base}/v1/issues/{missing}/comments"))
        .send()
        .await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn empty_body_returns_400() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();
    let issue_id = create_issue(&client, &base).await?;

    let resp = client
        .post(format!("{base}/v1/issues/{issue_id}/comments"))
        .json(&AddCommentRequest::new(String::new()))
        .send()
        .await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    Ok(())
}

#[tokio::test]
async fn whitespace_only_body_returns_400() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();
    let issue_id = create_issue(&client, &base).await?;

    let resp = client
        .post(format!("{base}/v1/issues/{issue_id}/comments"))
        .json(&AddCommentRequest::new("  \n\t  ".to_string()))
        .send()
        .await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    Ok(())
}

#[tokio::test]
async fn user_actor_attribution_round_trips() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();
    let issue_id = create_issue(&client, &base).await?;

    let added: AddCommentResponse = client
        .post(format!("{base}/v1/issues/{issue_id}/comments"))
        .json(&AddCommentRequest::new("posted by user".to_string()))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    match added.comment.actor {
        ActorRef::Authenticated { actor_id, .. } => match actor_id {
            ActorId::User(name) => assert_eq!(name.as_ref(), "test-creator"),
            other => panic!("expected User actor id, got {other:?}"),
        },
        other => panic!("expected Authenticated actor ref, got {other:?}"),
    }

    Ok(())
}

#[tokio::test]
async fn agent_actor_attribution_round_trips() -> anyhow::Result<()> {
    let handles = test_state_handles();
    add_agent_with_name(&handles, "swe").await;

    let agent_name = AgentName::try_new("swe").expect("static agent name");
    let (agent_actor, agent_token) = Actor::new_from_actor_id(
        ActorId::Agent(agent_name.clone()),
        Username::from("test-creator"),
        None,
    );
    register_actor_and_token(handles.store.as_ref(), &agent_actor, &agent_token, None).await?;

    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let user_client = test_client();
    let base = server.base_url();
    let issue_id = create_issue(&user_client, &base).await?;

    let agent_client = client_with_token(&agent_token);
    let added: AddCommentResponse = agent_client
        .post(format!("{base}/v1/issues/{issue_id}/comments"))
        .json(&AddCommentRequest::new("posted by agent".to_string()))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    match added.comment.actor {
        ActorRef::Authenticated { actor_id, .. } => match actor_id {
            ActorId::Agent(name) => assert_eq!(name.as_str(), "swe"),
            other => panic!("expected Agent actor id, got {other:?}"),
        },
        other => panic!("expected Authenticated actor ref, got {other:?}"),
    }

    Ok(())
}

#[tokio::test]
async fn limit_is_clamped_to_max_200() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();
    let issue_id = create_issue(&client, &base).await?;

    for i in 0..3 {
        client
            .post(format!("{base}/v1/issues/{issue_id}/comments"))
            .json(&AddCommentRequest::new(format!("c{i}")))
            .send()
            .await?
            .error_for_status()?;
    }

    let listed: ListCommentsResponse = client
        .get(format!("{base}/v1/issues/{issue_id}/comments"))
        .query(&[("limit", "9999")])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(listed.comments.len(), 3);
    assert!(listed.next_before_sequence.is_none());

    Ok(())
}
