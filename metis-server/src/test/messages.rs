use crate::{
    domain::{
        actors::{Actor, ActorRef},
        users::Username,
    },
    test_utils::{
        spawn_test_server, spawn_test_server_with_state, test_client, test_state_handles,
    },
};
use metis_common::{
    ActorId, IssueId,
    api::v1::messages::{ListMessagesResponse, SendMessageRequest, SendMessageResponse},
};
use reqwest::StatusCode;
use std::str::FromStr;

/// Helper to create a recipient actor (issue actor) and seed it in the store.
/// Returns the test server, client, recipient ActorId, and recipient actor name.
async fn setup_with_recipient() -> anyhow::Result<(
    crate::test_utils::TestServer,
    reqwest::Client,
    ActorId, // recipient actor id
    String,  // recipient actor name
)> {
    let handles = test_state_handles();
    let store = handles.store.clone();

    // Create a recipient actor (an issue actor)
    let issue_id = IssueId::from_str("i-testrec")?;
    let (recipient_actor, _recipient_token) =
        Actor::new_for_issue(issue_id, Username::from("test-creator"));
    let recipient_actor_name = recipient_actor.name();
    let recipient_actor_id = recipient_actor.actor_id.clone();
    store.add_actor(recipient_actor, &ActorRef::test()).await?;

    let server = spawn_test_server_with_state(handles.state, store).await?;
    let client = test_client();

    Ok((server, client, recipient_actor_id, recipient_actor_name))
}

#[tokio::test]
async fn send_message_creates_and_returns_versioned_response() -> anyhow::Result<()> {
    let (server, client, recipient_id, _recipient_name) = setup_with_recipient().await?;

    let response = client
        .post(format!("{}/v1/messages", server.base_url()))
        .json(&SendMessageRequest::new(
            recipient_id,
            "hello from test".to_string(),
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);

    let created: SendMessageResponse = response.json().await?;
    assert!(!created.message_id.as_ref().is_empty());
    assert_eq!(created.version, 1);
    assert_eq!(created.message.body, "hello from test");
    assert!(created.message.sender.is_some());

    Ok(())
}

#[tokio::test]
async fn send_message_returns_404_for_nonexistent_recipient() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let nonexistent = ActorId::Issue(IssueId::from_str("i-doesnotexist")?);

    let response = client
        .post(format!("{}/v1/messages", server.base_url()))
        .json(&SendMessageRequest::new(nonexistent, "hello".to_string()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn list_messages_returns_messages_in_descending_order() -> anyhow::Result<()> {
    let (server, client, recipient_id, recipient_name) = setup_with_recipient().await?;
    let base = server.base_url();

    // Send multiple messages
    let _: SendMessageResponse = client
        .post(format!("{base}/v1/messages"))
        .json(&SendMessageRequest::new(
            recipient_id.clone(),
            "first message".to_string(),
        ))
        .send()
        .await?
        .json()
        .await?;

    let _: SendMessageResponse = client
        .post(format!("{base}/v1/messages"))
        .json(&SendMessageRequest::new(
            recipient_id.clone(),
            "second message".to_string(),
        ))
        .send()
        .await?
        .json()
        .await?;

    let _: SendMessageResponse = client
        .post(format!("{base}/v1/messages"))
        .json(&SendMessageRequest::new(
            recipient_id,
            "third message".to_string(),
        ))
        .send()
        .await?
        .json()
        .await?;

    // List with recipient filter
    let list: ListMessagesResponse = client
        .get(format!("{base}/v1/messages?recipient={recipient_name}"))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(list.messages.len(), 3);
    // Most recent first
    assert_eq!(list.messages[0].message.body, "third message");
    assert_eq!(list.messages[1].message.body, "second message");
    assert_eq!(list.messages[2].message.body, "first message");

    // All messages should have version 1
    for msg in &list.messages {
        assert_eq!(msg.version, 1);
    }

    Ok(())
}

#[tokio::test]
async fn list_messages_without_filter_returns_all_messages() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let store = handles.store.clone();

    // Create two recipient actors
    let issue_id_1 = IssueId::from_str("i-recpa")?;
    let (actor1, _) = Actor::new_for_issue(issue_id_1, Username::from("test-creator"));
    store.add_actor(actor1.clone(), &ActorRef::test()).await?;

    let issue_id_2 = IssueId::from_str("i-recpb")?;
    let (actor2, _) = Actor::new_for_issue(issue_id_2, Username::from("test-creator"));
    store.add_actor(actor2.clone(), &ActorRef::test()).await?;

    let server = spawn_test_server_with_state(handles.state, store).await?;
    let client = test_client();
    let base = server.base_url();

    // Send messages to both recipients
    let _: SendMessageResponse = client
        .post(format!("{base}/v1/messages"))
        .json(&SendMessageRequest::new(
            actor1.actor_id.clone(),
            "msg to recipient 1".to_string(),
        ))
        .send()
        .await?
        .json()
        .await?;

    let _: SendMessageResponse = client
        .post(format!("{base}/v1/messages"))
        .json(&SendMessageRequest::new(
            actor2.actor_id.clone(),
            "msg to recipient 2".to_string(),
        ))
        .send()
        .await?
        .json()
        .await?;

    // List without any filter — should get all messages
    let list: ListMessagesResponse = client
        .get(format!("{base}/v1/messages"))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(list.messages.len(), 2);

    let bodies: Vec<&str> = list
        .messages
        .iter()
        .map(|m| m.message.body.as_str())
        .collect();
    assert!(bodies.contains(&"msg to recipient 1"));
    assert!(bodies.contains(&"msg to recipient 2"));

    Ok(())
}

#[tokio::test]
async fn list_messages_with_limit() -> anyhow::Result<()> {
    let (server, client, recipient_id, recipient_name) = setup_with_recipient().await?;
    let base = server.base_url();

    // Send 3 messages
    for body in &["first", "second", "third"] {
        let _: SendMessageResponse = client
            .post(format!("{base}/v1/messages"))
            .json(&SendMessageRequest::new(
                recipient_id.clone(),
                body.to_string(),
            ))
            .send()
            .await?
            .json()
            .await?;
    }

    // List with limit=1
    let list: ListMessagesResponse = client
        .get(format!(
            "{base}/v1/messages?recipient={recipient_name}&limit=1"
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(list.messages.len(), 1);
    assert_eq!(list.messages[0].message.body, "third");

    Ok(())
}

#[tokio::test]
async fn wait_for_message_returns_on_new_message() -> anyhow::Result<()> {
    let (server, client, recipient_id, recipient_name) = setup_with_recipient().await?;
    let base_url = server.base_url();

    // Start the wait in a background task
    let wait_client = test_client();
    let wait_url = format!("{base_url}/v1/messages/wait?recipient={recipient_name}&timeout=10");
    let wait_handle = tokio::spawn(async move {
        let response: ListMessagesResponse = wait_client
            .get(wait_url)
            .send()
            .await
            .expect("wait request should succeed")
            .json()
            .await
            .expect("wait response should parse");
        response
    });

    // Give the wait request time to establish
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Send a message — this should unblock the wait
    let _: SendMessageResponse = client
        .post(format!("{base_url}/v1/messages"))
        .json(&SendMessageRequest::new(
            recipient_id,
            "wake up!".to_string(),
        ))
        .send()
        .await?
        .json()
        .await?;

    // The wait should complete with the new message
    let wait_result = wait_handle.await?;
    assert_eq!(wait_result.messages.len(), 1);
    assert_eq!(wait_result.messages[0].message.body, "wake up!");

    Ok(())
}

#[tokio::test]
async fn wait_for_message_times_out_with_empty_response() -> anyhow::Result<()> {
    let (server, client, _recipient_id, recipient_name) = setup_with_recipient().await?;
    let base = server.base_url();

    let start = std::time::Instant::now();
    let wait_result: ListMessagesResponse = client
        .get(format!(
            "{base}/v1/messages/wait?recipient={recipient_name}&timeout=1"
        ))
        .send()
        .await?
        .json()
        .await?;
    let elapsed = start.elapsed();

    assert!(wait_result.messages.is_empty());
    // Should have waited approximately 1 second
    assert!(
        elapsed.as_millis() >= 800,
        "wait should block for approximately the timeout duration"
    );

    Ok(())
}

#[tokio::test]
async fn wait_returns_existing_messages_after_cursor() -> anyhow::Result<()> {
    let (server, client, recipient_id, recipient_name) = setup_with_recipient().await?;
    let base = server.base_url();

    // Send two messages
    let msg1: SendMessageResponse = client
        .post(format!("{base}/v1/messages"))
        .json(&SendMessageRequest::new(
            recipient_id.clone(),
            "older message".to_string(),
        ))
        .send()
        .await?
        .json()
        .await?;

    let _msg2: SendMessageResponse = client
        .post(format!("{base}/v1/messages"))
        .json(&SendMessageRequest::new(
            recipient_id,
            "newer message".to_string(),
        ))
        .send()
        .await?
        .json()
        .await?;

    // Wait with after=msg1 — should return msg2 immediately
    let wait_result: ListMessagesResponse = client
        .get(format!(
            "{base}/v1/messages/wait?recipient={recipient_name}&after={}&timeout=1",
            msg1.message_id
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(wait_result.messages.len(), 1);
    assert_eq!(wait_result.messages[0].message.body, "newer message");

    Ok(())
}

#[tokio::test]
async fn list_messages_requires_authentication() -> anyhow::Result<()> {
    let (server, _client, _recipient_id, recipient_name) = setup_with_recipient().await?;
    let base = server.base_url();

    // List messages without authentication — should be rejected
    let unauthenticated_client = reqwest::Client::new();
    let response = unauthenticated_client
        .get(format!("{base}/v1/messages?recipient={recipient_name}"))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    Ok(())
}

#[tokio::test]
async fn list_messages_allows_any_authenticated_actor() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let store = handles.store.clone();

    // Create a recipient actor
    let issue_id = IssueId::from_str("i-recpx")?;
    let (recipient_actor, _) = Actor::new_for_issue(issue_id, Username::from("test-creator"));
    let recipient_name = recipient_actor.name();
    let recipient_id = recipient_actor.actor_id.clone();
    store.add_actor(recipient_actor, &ActorRef::test()).await?;

    // Create a second (different) actor with its own auth token
    let issue_id_2 = IssueId::from_str("i-other")?;
    let (other_actor, other_token) =
        Actor::new_for_issue(issue_id_2, Username::from("other-creator"));
    store.add_actor(other_actor, &ActorRef::test()).await?;

    let server = spawn_test_server_with_state(handles.state, store).await?;
    let client = test_client();
    let base = server.base_url();

    // Send a message as the default test actor to the recipient
    let _: SendMessageResponse = client
        .post(format!("{base}/v1/messages"))
        .json(&SendMessageRequest::new(
            recipient_id,
            "test message".to_string(),
        ))
        .send()
        .await?
        .json()
        .await?;

    // List messages as the other actor — should succeed and see the message
    let other_client = {
        let mut headers = reqwest::header::HeaderMap::new();
        let auth_value = format!("Bearer {other_token}");
        headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&auth_value)?,
        );
        reqwest::Client::builder()
            .default_headers(headers)
            .build()?
    };

    let list: ListMessagesResponse = other_client
        .get(format!("{base}/v1/messages?recipient={recipient_name}"))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(list.messages.len(), 1);
    assert_eq!(list.messages[0].message.body, "test message");

    Ok(())
}
