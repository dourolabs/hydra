use crate::test_utils::{spawn_test_server, test_client_without_auth};
use reqwest::StatusCode;

#[tokio::test]
async fn protected_routes_require_auth() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client_without_auth();

    let response = client
        .get(format!("{}/v1/issues", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
async fn public_routes_accept_requests_without_auth() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client_without_auth();

    let response = client
        .get(format!("{}/health", server.base_url()))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    Ok(())
}

#[cfg(feature = "github")]
#[tokio::test]
async fn github_public_routes_accept_requests_without_auth() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client_without_auth();

    let response = client
        .get(format!("{}/v1/github/app/client-id", server.base_url()))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    Ok(())
}
