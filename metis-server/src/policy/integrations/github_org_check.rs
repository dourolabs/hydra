use octocrab::Octocrab;
use serde::Deserialize;

use crate::policy::PolicyViolation;

/// Check that a GitHub user belongs to at least one of the allowed
/// organizations. When `allowed_orgs` is empty the check is a no-op (all
/// users are permitted).
///
/// This is a login-time policy check that requires a GitHub API call, so it
/// lives in the integrations module rather than as a standard `Restriction`
/// (which operates on store mutations).
pub async fn check_github_org_membership(
    github_client: &Octocrab,
    username: &str,
    allowed_orgs: &[String],
) -> Result<(), PolicyViolation> {
    if allowed_orgs.is_empty() {
        return Ok(());
    }

    #[derive(Deserialize)]
    struct GithubOrg {
        login: String,
    }

    let orgs: Vec<GithubOrg> =
        github_client
            .get("/user/orgs", None::<&()>)
            .await
            .map_err(|err| PolicyViolation {
                policy_name: "github_org_check".to_string(),
                message: format!("Failed to fetch GitHub organizations for user {username}: {err}"),
            })?;

    let is_allowed = orgs.iter().any(|org| {
        allowed_orgs
            .iter()
            .any(|allowed| org.login.eq_ignore_ascii_case(allowed))
    });

    if !is_allowed {
        let org_list = allowed_orgs.join(", ");
        return Err(PolicyViolation {
            policy_name: "github_org_check".to_string(),
            message: format!(
                "User {username} is not a member of any allowed organization. \
                 Allowed orgs: {org_list}. Contact your administrator for access."
            ),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    fn build_github_client(base_url: String) -> Octocrab {
        Octocrab::builder()
            .base_uri(base_url)
            .unwrap()
            .personal_token("test-token".to_string())
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn allows_user_in_allowed_org() {
        let server = MockServer::start_async().await;
        server.mock(|when, then| {
            when.method(GET).path("/user/orgs");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(serde_json::json!([{"login": "metis"}]));
        });

        let client = build_github_client(server.base_url());
        let result = check_github_org_membership(&client, "alice", &["metis".to_string()]).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn allows_case_insensitive_match() {
        let server = MockServer::start_async().await;
        server.mock(|when, then| {
            when.method(GET).path("/user/orgs");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(serde_json::json!([{"login": "Metis"}]));
        });

        let client = build_github_client(server.base_url());
        let result = check_github_org_membership(&client, "alice", &["metis".to_string()]).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn rejects_user_not_in_allowed_org() {
        let server = MockServer::start_async().await;
        server.mock(|when, then| {
            when.method(GET).path("/user/orgs");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(serde_json::json!([{"login": "other-org"}]));
        });

        let client = build_github_client(server.base_url());
        let result = check_github_org_membership(&client, "alice", &["metis".to_string()]).await;
        let violation = result.unwrap_err();
        assert_eq!(violation.policy_name, "github_org_check");
        assert!(violation.message.contains("alice"));
        assert!(violation.message.contains("metis"));
        assert!(violation.message.contains("Contact your administrator"));
    }

    #[tokio::test]
    async fn skips_check_when_no_orgs_configured() {
        // No mock server needed since no API call should be made.
        let client = build_github_client("http://localhost:1".to_string());
        let result = check_github_org_membership(&client, "alice", &[]).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn rejects_when_user_has_no_orgs() {
        let server = MockServer::start_async().await;
        server.mock(|when, then| {
            when.method(GET).path("/user/orgs");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(serde_json::json!([]));
        });

        let client = build_github_client(server.base_url());
        let result = check_github_org_membership(&client, "bob", &["metis".to_string()]).await;
        let violation = result.unwrap_err();
        assert_eq!(violation.policy_name, "github_org_check");
        assert!(violation.message.contains("bob"));
    }
}
