use crate::{
    domain::{
        actors::{Actor, ActorRef},
        users::Username,
    },
    test_utils::{spawn_test_server_with_state, test_client, test_state_handles},
};
use metis_common::{
    ActorId, IssueId,
    api::v1::messages::{ListMessagesResponse, SendMessageRequest, SendMessageResponse},
};
use reqwest::{Client, header};
use std::str::FromStr;

/// Create an HTTP client authenticated with the given bearer token.
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

/// Full end-to-end messaging flow:
///
/// 1. User sends a message to issue-agent.
/// 2. Issue-agent lists messages and sees the user's message.
/// 3. Issue-agent sends a reply.
/// 4. User long-polls and receives the reply.
/// 5. Verify ordering (most recent first in list).
/// 6. Verify versioned fields (version, timestamp, creation_time).
/// 7. Verify a third actor cannot see the conversation.
#[tokio::test]
async fn messaging_e2e_full_conversation_flow() -> anyhow::Result<()> {
    // ── Setup ──────────────────────────────────────────────────────────
    let handles = test_state_handles();
    let store = handles.store.clone();

    // The default test actor acts as the "user" (a task-based actor).
    let user_client = test_client();

    // Create the issue-agent actor.
    let issue_id = IssueId::from_str("i-eteagent")?;
    let (agent_actor, agent_token) = Actor::new_for_issue(issue_id, Username::from("test-creator"));
    let agent_actor_id = agent_actor.actor_id.clone();
    let agent_actor_name = agent_actor.name();
    store.add_actor(agent_actor, &ActorRef::test()).await?;

    // Create a third actor that should NOT be able to see the conversation.
    let third_issue_id = IssueId::from_str("i-etethird")?;
    let (third_actor, third_token) =
        Actor::new_for_issue(third_issue_id, Username::from("test-creator"));
    store.add_actor(third_actor, &ActorRef::test()).await?;

    let agent_client = client_with_token(&agent_token);
    let third_client = client_with_token(&third_token);

    let server = spawn_test_server_with_state(handles.state, store).await?;
    let base = server.base_url();

    // ── Step 1: User sends a message to issue-agent ───────────────────
    let send_resp: SendMessageResponse = user_client
        .post(format!("{base}/v1/messages"))
        .json(&SendMessageRequest::new(
            agent_actor_id.clone(),
            "Hello agent, please help me.".to_string(),
        ))
        .send()
        .await?
        .json()
        .await?;

    assert!(!send_resp.message_id.as_ref().is_empty());
    assert_eq!(send_resp.version, 1);
    assert_eq!(send_resp.message.body, "Hello agent, please help me.");
    assert!(!send_resp.message.conversation_id.is_empty());
    let user_msg_id = send_resp.message_id.clone();

    // ── Step 2: Issue-agent lists messages and sees the user's message ─
    // The agent uses participant= <user actor name> to scope the conversation.
    // The default test actor's name is its ActorId string representation.
    let user_actor_name = {
        // Derive the user actor name from the send response sender field.
        send_resp.message.sender.to_string()
    };

    let agent_list: ListMessagesResponse = agent_client
        .get(format!("{base}/v1/messages?participant={user_actor_name}"))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(agent_list.messages.len(), 1);
    assert_eq!(
        agent_list.messages[0].message.body,
        "Hello agent, please help me."
    );

    // Verify versioned fields on the listed message.
    let listed_msg = &agent_list.messages[0];
    assert_eq!(listed_msg.version, 1);
    assert_eq!(listed_msg.message_id, user_msg_id);
    // creation_time and timestamp should be populated (not epoch zero).
    assert!(
        listed_msg.creation_time.timestamp() > 0,
        "creation_time should be set"
    );
    assert!(
        listed_msg.timestamp.timestamp() > 0,
        "timestamp should be set"
    );

    // ── Step 3: Issue-agent sends a reply to the user ──────────────────
    // The agent needs to address the user's ActorId.
    let user_actor_id: ActorId = send_resp.message.sender.clone();

    let reply_resp: SendMessageResponse = agent_client
        .post(format!("{base}/v1/messages"))
        .json(&SendMessageRequest::new(
            user_actor_id,
            "I'm on it! Working now.".to_string(),
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(reply_resp.version, 1);
    assert_eq!(reply_resp.message.body, "I'm on it! Working now.");
    // Both messages should share the same conversation_id.
    assert_eq!(
        reply_resp.message.conversation_id,
        send_resp.message.conversation_id
    );

    // ── Step 4: User long-polls and receives the agent's reply ─────────
    // Use the user's first message id as the "after" cursor so we only get
    // messages newer than it.
    let wait_result: ListMessagesResponse = user_client
        .get(format!(
            "{base}/v1/messages/wait?participant={agent_actor_name}&after={user_msg_id}&timeout=5"
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(wait_result.messages.len(), 1);
    assert_eq!(
        wait_result.messages[0].message.body,
        "I'm on it! Working now."
    );
    assert_eq!(wait_result.messages[0].message_id, reply_resp.message_id);

    // ── Step 5: Verify ordering (most recent first in list) ────────────
    let user_list: ListMessagesResponse = user_client
        .get(format!("{base}/v1/messages?participant={agent_actor_name}"))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(user_list.messages.len(), 2);
    // Most recent first.
    assert_eq!(
        user_list.messages[0].message.body,
        "I'm on it! Working now."
    );
    assert_eq!(
        user_list.messages[1].message.body,
        "Hello agent, please help me."
    );

    // ── Step 6: Verify versioned fields on all messages ────────────────
    for msg in &user_list.messages {
        assert_eq!(msg.version, 1, "initial version should be 1");
        assert!(
            msg.creation_time.timestamp() > 0,
            "creation_time must be populated"
        );
        assert!(msg.timestamp.timestamp() > 0, "timestamp must be populated");
        assert!(
            !msg.message_id.as_ref().is_empty(),
            "message_id must be populated"
        );
        assert!(
            !msg.message.conversation_id.is_empty(),
            "conversation_id must be populated"
        );
    }

    // ── Step 7: Third actor cannot see the conversation ────────────────
    // The third actor has no messages in this conversation.
    let third_list: ListMessagesResponse = third_client
        .get(format!("{base}/v1/messages?participant={agent_actor_name}"))
        .send()
        .await?
        .json()
        .await?;

    assert!(
        third_list.messages.is_empty(),
        "third actor must not see messages between user and agent"
    );

    // Also verify the third actor cannot see anything in the "all conversations" view
    // related to the user-agent conversation.
    let third_all: ListMessagesResponse = third_client
        .get(format!("{base}/v1/messages"))
        .send()
        .await?
        .json()
        .await?;

    assert!(
        third_all.messages.is_empty(),
        "third actor must not see any messages at all"
    );

    Ok(())
}

/// End-to-end: long-poll wait unblocks on a new message delivered asynchronously.
///
/// This tests the event-bus driven long-poll (rather than cursor-based
/// early return) where the wait starts before the message is sent.
#[tokio::test]
async fn messaging_e2e_wait_long_poll_unblocks_on_new_message() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let store = handles.store.clone();

    let user_client = test_client();

    let issue_id = IssueId::from_str("i-etewait")?;
    let (agent_actor, agent_token) = Actor::new_for_issue(issue_id, Username::from("test-creator"));
    let agent_actor_id = agent_actor.actor_id.clone();
    let agent_actor_name = agent_actor.name();
    store.add_actor(agent_actor, &ActorRef::test()).await?;

    let agent_client = client_with_token(&agent_token);

    let server = spawn_test_server_with_state(handles.state, store).await?;
    let base = server.base_url();

    // User sends a first message (to establish the conversation).
    let first: SendMessageResponse = user_client
        .post(format!("{base}/v1/messages"))
        .json(&SendMessageRequest::new(
            agent_actor_id.clone(),
            "initial ping".to_string(),
        ))
        .send()
        .await?
        .json()
        .await?;

    let user_actor_id = first.message.sender.clone();

    // Agent starts long-polling for new messages *before* the user sends the
    // next one, using the first message as the "after" cursor.
    let wait_base = base.clone();
    let wait_user_name = first.message.sender.to_string();
    let after_id = first.message_id.to_string();
    let wait_handle = tokio::spawn(async move {
        let agent_wait_client = client_with_token(&agent_token);
        let resp: ListMessagesResponse = agent_wait_client
            .get(format!(
                "{wait_base}/v1/messages/wait?participant={wait_user_name}&after={after_id}&timeout=10"
            ))
            .send()
            .await
            .expect("wait should succeed")
            .json()
            .await
            .expect("parse wait response");
        resp
    });

    // Give the wait request time to establish.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // User sends another message — this should unblock the agent's wait.
    let _second: SendMessageResponse = user_client
        .post(format!("{base}/v1/messages"))
        .json(&SendMessageRequest::new(
            agent_actor_id,
            "follow-up question".to_string(),
        ))
        .send()
        .await?
        .json()
        .await?;

    let wait_result = wait_handle.await?;
    assert_eq!(wait_result.messages.len(), 1);
    assert_eq!(wait_result.messages[0].message.body, "follow-up question");

    // Verify the agent can reply and the user sees the full conversation.
    let _reply: SendMessageResponse = agent_client
        .post(format!("{base}/v1/messages"))
        .json(&SendMessageRequest::new(
            user_actor_id,
            "got your follow-up".to_string(),
        ))
        .send()
        .await?
        .json()
        .await?;

    let user_list: ListMessagesResponse = user_client
        .get(format!("{base}/v1/messages?participant={agent_actor_name}"))
        .send()
        .await?
        .json()
        .await?;

    // Should have 3 messages total: initial ping, follow-up, reply.
    assert_eq!(user_list.messages.len(), 3);
    assert_eq!(user_list.messages[0].message.body, "got your follow-up");
    assert_eq!(user_list.messages[1].message.body, "follow-up question");
    assert_eq!(user_list.messages[2].message.body, "initial ping");

    Ok(())
}
