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
async fn whoami_returns_task_identity() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let task_id = SessionId::new();
    let (actor, auth_token) = Actor::new_for_session(task_id.clone(), Username::from("creator"));
    handles
        .store
        .as_ref()
        .add_actor(actor.clone(), &ActorRef::test())
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
        ActorIdentity::Session {
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
            panic!("expected task identity, got {other:?}");
        }
    }

    // Sanity check: ActorId on the actor is Session-typed.
    assert!(matches!(actor.actor_id, ActorId::Session(_)));

    Ok(())
}
