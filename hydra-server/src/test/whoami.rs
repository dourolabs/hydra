use crate::domain::actors::{Actor, ActorRef};
use crate::domain::users::Username;
use crate::test::{
    github_user_response, spawn_test_server_with_state, test_state_handles,
    test_state_with_github_api_base_url,
};
use httpmock::prelude::*;
use hydra_common::api::v1::whoami::{ActorIdentity, WhoAmIResponse};
use hydra_common::{ActorId, SessionId};
use reqwest::{Client, StatusCode, header};

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
async fn whoami_returns_user_identity() -> anyhow::Result<()> {
    let github_server = MockServer::start_async().await;
    let _mock = github_server.mock(|when, then| {
        when.method(GET).path("/user");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(github_user_response("octo", 42));
    });

    let handles = test_state_with_github_api_base_url(github_server.base_url());
    let token = handles
        .state
        .login_with_github_token(
            "gh-token".to_string(),
            "gh-refresh".to_string(),
            ActorRef::test(),
        )
        .await?
        .login_token;

    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = client_with_token(&token);
    let response = client
        .get(format!("{}/v1/whoami", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);

    let body: WhoAmIResponse = response.json().await?;
    match body.actor {
        ActorIdentity::User { username } => {
            assert_eq!(username.as_str(), "octo");
        }
        other => {
            panic!("expected user identity, got {other:?}");
        }
    }

    Ok(())
}

#[tokio::test]
async fn whoami_returns_adhoc_identity_for_adhoc_actor() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let task_id = SessionId::new();
    let (actor, auth_token) = Actor::new_from_actor_id(
        ActorId::Adhoc(task_id.clone()),
        Username::from("creator"),
        None,
    );
    crate::test_utils::register_actor_and_token(
        handles.store.as_ref(),
        &actor,
        &auth_token,
        Some(&task_id),
    )
    .await?;

    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = client_with_token(&auth_token);

    let response = client
        .get(format!("{}/v1/whoami", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);

    let body: WhoAmIResponse = response.json().await?;
    match body.actor {
        ActorIdentity::Adhoc {
            session_id,
            creator,
        } => {
            assert_eq!(session_id, task_id);
            assert_eq!(
                creator.as_str(),
                "creator",
                "creator should match the actor's creator"
            );
        }
        other => {
            panic!("expected adhoc identity, got {other:?}");
        }
    }

    // Sanity check: ActorId on the actor is Adhoc-typed.
    assert!(matches!(actor.actor_id, ActorId::Adhoc(_)));

    Ok(())
}

// `whoami` must surface the typed `Agent` / `Adhoc` variants stamped by
// `create_actor_for_job` rather than rejecting the request. The tests
// below exercise both arms end-to-end through the HTTP handler.

#[tokio::test]
async fn whoami_returns_agent_identity_for_agent_session() -> anyhow::Result<()> {
    use crate::app::test_helpers::sample_task;
    use crate::domain::sessions::AgentConfig;
    use hydra_common::api::v1::agents::AgentName;

    let handles = test_state_handles();
    let agent_name = AgentName::try_new("swe").unwrap();
    let mut task = sample_task();
    task.agent_config = AgentConfig::new(Some(agent_name.clone()), None, None, None);

    let (session_id, _) = handles
        .store
        .add_session(task, chrono::Utc::now(), &ActorRef::test())
        .await?;

    let (_actor, auth_token) = handles
        .state
        .create_actor_for_job(session_id, ActorRef::test())
        .await?;

    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = client_with_token(&auth_token);
    let response = client
        .get(format!("{}/v1/whoami", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: WhoAmIResponse = response.json().await?;
    match body.actor {
        ActorIdentity::Agent { name, creator } => {
            assert_eq!(name, agent_name);
            assert_eq!(creator.as_str(), "test-creator");
        }
        other => panic!("expected agent identity, got {other:?}"),
    }
    Ok(())
}

#[tokio::test]
async fn whoami_returns_adhoc_identity_for_adhoc_session() -> anyhow::Result<()> {
    use crate::app::test_helpers::sample_task;

    let handles = test_state_handles();
    // `sample_task` already leaves agent_config.agent_name = None, so
    // routing through `create_actor_for_job` yields the Adhoc arm.
    let task = sample_task();
    let (session_id, _) = handles
        .store
        .add_session(task, chrono::Utc::now(), &ActorRef::test())
        .await?;

    let (_actor, auth_token) = handles
        .state
        .create_actor_for_job(session_id.clone(), ActorRef::test())
        .await?;

    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = client_with_token(&auth_token);
    let response = client
        .get(format!("{}/v1/whoami", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: WhoAmIResponse = response.json().await?;
    match body.actor {
        ActorIdentity::Adhoc {
            session_id: returned_id,
            creator,
        } => {
            assert_eq!(returned_id, session_id);
            assert_eq!(creator.as_str(), "test-creator");
        }
        other => panic!("expected adhoc identity, got {other:?}"),
    }
    Ok(())
}
