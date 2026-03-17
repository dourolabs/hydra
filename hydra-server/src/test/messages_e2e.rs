use crate::{
    domain::{
        actors::{Actor, ActorRef},
        users::Username,
    },
    test_utils::{spawn_test_server_with_state, test_client, test_state_handles},
};
use hydra_common::{
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
/// 4. User receives the reply via the receive endpoint.
/// 5. Verify ordering (most recent first in list).
/// 6. Verify versioned fields (version, timestamp, creation_time).
/// 7. Verify filtering by a third actor's recipient name returns no results (no messages addressed to them).
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

    // Create a third actor — no messages will be addressed to it, so recipient filtering returns empty.
    let third_issue_id = IssueId::from_str("i-etethird")?;
    let (third_actor, third_token) =
        Actor::new_for_issue(third_issue_id, Username::from("test-creator"));
    let third_actor_name = third_actor.name();
    store.add_actor(third_actor, &ActorRef::test()).await?;

    let agent_client = client_with_token(&agent_token);

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
    assert!(send_resp.message.sender.is_some());
    let user_msg_id = send_resp.message_id.clone();

    // ── Step 2: Issue-agent lists messages and sees the user's message ─
    // Use the sender filter to scope by the user who sent the message.
    let user_actor_name = send_resp.message.sender.as_ref().unwrap().to_string();

    let agent_list: ListMessagesResponse = agent_client
        .get(format!(
            "{base}/v1/messages?recipient={agent_actor_name}&sender={user_actor_name}"
        ))
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
    assert!(
        listed_msg.creation_time.timestamp() > 0,
        "creation_time should be set"
    );
    assert!(
        listed_msg.timestamp.timestamp() > 0,
        "timestamp should be set"
    );

    // ── Step 3: Issue-agent sends a reply to the user ──────────────────
    let user_actor_id: ActorId = send_resp.message.sender.clone().unwrap();

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

    // ── Step 4: User receives the agent's reply via the receive endpoint ─
    let receive_result: ListMessagesResponse = user_client
        .get(format!(
            "{base}/v1/messages/receive?sender={agent_actor_name}&timeout=5"
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(receive_result.messages.len(), 1);
    assert_eq!(
        receive_result.messages[0].message.body,
        "I'm on it! Working now."
    );
    assert_eq!(receive_result.messages[0].message_id, reply_resp.message_id);
    // Receive returns the original unread version of the message
    assert!(
        !receive_result.messages[0].message.is_read,
        "received messages should return original unread version"
    );

    // ── Step 5: Verify ordering (most recent first in list) ────────────
    let user_list: ListMessagesResponse = user_client
        .get(format!("{base}/v1/messages?recipient={agent_actor_name}"))
        .send()
        .await?
        .json()
        .await?;

    // This should return messages where the agent is the recipient
    // (i.e., the user sent them), which is 1 message.
    assert_eq!(user_list.messages.len(), 1);
    assert_eq!(
        user_list.messages[0].message.body,
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
    }

    // ── Step 7: Third actor filters by own recipient name — gets empty results ──
    // The third actor is not the sender or recipient of any messages.
    let third_client = client_with_token(&third_token);
    let third_list: ListMessagesResponse = third_client
        .get(format!("{base}/v1/messages?recipient={third_actor_name}"))
        .send()
        .await?
        .json()
        .await?;

    assert!(
        third_list.messages.is_empty(),
        "no messages have the third actor as recipient"
    );

    Ok(())
}

/// End-to-end: receive long-polls and unblocks on a new message delivered asynchronously.
///
/// This tests the event-bus driven long-poll (rather than existing unread messages)
/// where the receive starts before the message is sent.
#[tokio::test]
async fn messaging_e2e_receive_long_poll_unblocks_on_new_message() -> anyhow::Result<()> {
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

    // User sends a first message — we mark it as already read so it doesn't
    // interfere with the agent's receive call below.
    let first: SendMessageResponse = user_client
        .post(format!("{base}/v1/messages"))
        .json(&{
            let mut req =
                SendMessageRequest::new(agent_actor_id.clone(), "initial ping".to_string());
            req.is_read = true;
            req
        })
        .send()
        .await?
        .json()
        .await?;

    let user_actor_id = first.message.sender.clone().unwrap();
    let user_actor_name = user_actor_id.to_string();

    // Agent starts long-polling via receive *before* the user sends the
    // next message. Because the first message is already read, the receive
    // endpoint has no unread messages and will long-poll.
    let wait_base = base.clone();
    let wait_user_name = user_actor_name.clone();
    let wait_handle = tokio::spawn(async move {
        let agent_wait_client = client_with_token(&agent_token);
        let resp: ListMessagesResponse = agent_wait_client
            .get(format!(
                "{wait_base}/v1/messages/receive?sender={wait_user_name}&timeout=10"
            ))
            .send()
            .await
            .expect("receive should succeed")
            .json()
            .await
            .expect("parse receive response");
        resp
    });

    // Give the receive request time to establish.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // User sends another message — this should unblock the agent's receive.
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

    let receive_result = wait_handle.await?;
    assert_eq!(receive_result.messages.len(), 1);
    assert_eq!(
        receive_result.messages[0].message.body,
        "follow-up question"
    );
    // Long-poll receive returns the original unread version of the message
    assert!(
        !receive_result.messages[0].message.is_read,
        "received message should return original unread version"
    );

    // Verify the agent can reply and both see the messages.
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

    // List messages where agent is recipient
    let user_list: ListMessagesResponse = user_client
        .get(format!("{base}/v1/messages?recipient={agent_actor_name}"))
        .send()
        .await?
        .json()
        .await?;

    // Should have 2 messages to agent: initial ping, follow-up.
    assert_eq!(user_list.messages.len(), 2);
    assert_eq!(user_list.messages[0].message.body, "follow-up question");
    assert_eq!(user_list.messages[1].message.body, "initial ping");

    // List messages where user is recipient (agent's reply)
    let reply_list: ListMessagesResponse = user_client
        .get(format!("{base}/v1/messages?recipient={user_actor_name}"))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(reply_list.messages.len(), 1);
    assert_eq!(reply_list.messages[0].message.body, "got your follow-up");

    Ok(())
}

/// End-to-end: receive endpoint returns original unread messages and marks them as read.
///
/// 1. User sends messages to an issue-agent.
/// 2. Agent calls receive — gets original unread messages (is_read=false), server marks them as read.
/// 3. Agent lists messages again — all messages now have is_read=true.
/// 4. Agent calls receive again — no unread messages, times out with empty response.
#[tokio::test]
async fn messaging_e2e_receive_marks_as_read() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let store = handles.store.clone();

    let user_client = test_client();

    let issue_id = IssueId::from_str("i-eterecv")?;
    let (agent_actor, agent_token) = Actor::new_for_issue(issue_id, Username::from("test-creator"));
    let agent_actor_id = agent_actor.actor_id.clone();
    store.add_actor(agent_actor, &ActorRef::test()).await?;

    let agent_client = client_with_token(&agent_token);

    let server = spawn_test_server_with_state(handles.state, store).await?;
    let base = server.base_url();

    // ── Step 1: User sends two messages to the agent ─────────────────
    user_client
        .post(format!("{base}/v1/messages"))
        .json(&SendMessageRequest::new(
            agent_actor_id.clone(),
            "first message".to_string(),
        ))
        .send()
        .await?
        .error_for_status()?;

    user_client
        .post(format!("{base}/v1/messages"))
        .json(&SendMessageRequest::new(
            agent_actor_id.clone(),
            "second message".to_string(),
        ))
        .send()
        .await?
        .error_for_status()?;

    // ── Step 2: Agent receives messages — returns original unread versions ─
    let receive_resp: ListMessagesResponse = agent_client
        .get(format!("{base}/v1/messages/receive?timeout=5"))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(receive_resp.messages.len(), 2);
    // Receive returns in ascending order (oldest first)
    assert_eq!(receive_resp.messages[0].message.body, "first message");
    assert_eq!(receive_resp.messages[1].message.body, "second message");
    for msg in &receive_resp.messages {
        assert!(
            !msg.message.is_read,
            "received messages should return original unread version"
        );
    }

    // ── Step 3: Agent lists messages — all should have is_read=true ──
    let agent_actor_name = agent_actor_id.to_string();
    let after_list: ListMessagesResponse = agent_client
        .get(format!("{base}/v1/messages?recipient={agent_actor_name}"))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(after_list.messages.len(), 2);
    for msg in &after_list.messages {
        assert!(msg.message.is_read, "messages should be marked as read");
    }

    // ── Step 4: Agent receives again — no unread messages, should time out ─
    let second_receive: ListMessagesResponse = agent_client
        .get(format!("{base}/v1/messages/receive?timeout=1"))
        .send()
        .await?
        .json()
        .await?;

    assert!(
        second_receive.messages.is_empty(),
        "no unread messages, should return empty on timeout"
    );

    Ok(())
}
