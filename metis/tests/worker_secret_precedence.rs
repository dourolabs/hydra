mod harness;

use anyhow::Result;
use metis_server::domain::users::Username;
use std::str::FromStr;

/// Verify that the worker receives user-set CLAUDE_CODE_OAUTH_TOKEN from the
/// API response (not the server config fallback). This confirms that the
/// get_job_context endpoint resolves user secrets and the worker uses the API
/// response as the single source of truth.
#[tokio::test]
async fn worker_receives_user_secret_from_api() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/secrets-test")
        .build()
        .await?;

    // Encrypt and store a user secret for the default user.
    let secret_manager = &harness.state().secret_manager;
    let encrypted = secret_manager.encrypt("user-oauth-token-value")?;
    let username = Username::from("default");
    harness
        .store()
        .set_user_secret(&username, "CLAUDE_CODE_OAUTH_TOKEN", &encrypted, false)
        .await?;

    let user = harness.default_user();
    let repo = metis_common::RepoName::from_str("acme/secrets-test")?;
    let issue_id = user.create_issue("test secret precedence").await?;
    let job_id = user
        .create_session_for_issue(&repo, "test secret precedence", &issue_id)
        .await?;

    // The worker command prints CLAUDE_CODE_OAUTH_TOKEN so we can verify it.
    let result = harness
        .run_worker(
            &job_id,
            vec!["bash -c 'echo OAUTH_TOKEN=$CLAUDE_CODE_OAUTH_TOKEN'"],
        )
        .await?;

    assert!(
        !result.outputs.is_empty(),
        "expected at least one command output"
    );
    let output = &result.outputs[0].stdout;
    assert!(
        output.contains("OAUTH_TOKEN=user-oauth-token-value"),
        "expected user secret in worker env, got: {output}"
    );

    Ok(())
}
