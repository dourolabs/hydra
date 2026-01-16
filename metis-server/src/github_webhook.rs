use crate::{app::ServiceState, config::ResolvedGithubWebhookConfig};
use anyhow::{Context, Result};
use octocrab::Octocrab;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{info, warn};

#[derive(Debug, Clone)]
pub struct GithubWebhookRegistration {
    pub id: u64,
    pub created: bool,
}

#[derive(Debug, Deserialize)]
struct GithubWebhookResponse {
    id: u64,
    config: GithubWebhookResponseConfig,
}

#[derive(Debug, Deserialize)]
struct GithubWebhookResponseConfig {
    url: Option<String>,
}

#[derive(Debug, Serialize)]
struct GithubWebhookCreateRequest {
    name: String,
    active: bool,
    events: Vec<String>,
    config: GithubWebhookCreateConfig,
}

#[derive(Debug, Serialize)]
struct GithubWebhookCreateConfig {
    url: String,
    content_type: String,
    secret: String,
    insecure_ssl: String,
}

pub async fn register_configured_webhooks(
    service_state: &ServiceState,
    webhook_config: &ResolvedGithubWebhookConfig,
) -> Result<HashMap<String, GithubWebhookRegistration>> {
    let mut registrations = HashMap::new();

    for (name, repo) in &service_state.repositories {
        let Some(token) = repo.github_token.as_deref() else {
            warn!(repository = %name, "skipping GitHub webhook registration without token");
            continue;
        };
        let Some((owner, repo_name)) = parse_github_remote(&repo.remote_url) else {
            warn!(repository = %name, remote = %repo.remote_url, "skipping non-GitHub remote");
            continue;
        };

        let client = github_client(token)?;
        let registration =
            register_repo_webhook(&client, &owner, &repo_name, webhook_config).await?;
        info!(
            repository = %name,
            hook_id = registration.id,
            created = registration.created,
            "configured GitHub webhook"
        );
        registrations.insert(name.clone(), registration);
    }

    if !registrations.is_empty() {
        info!(count = registrations.len(), "registered GitHub webhooks");
    }

    Ok(registrations)
}

async fn register_repo_webhook(
    client: &Octocrab,
    owner: &str,
    repo: &str,
    webhook_config: &ResolvedGithubWebhookConfig,
) -> Result<GithubWebhookRegistration> {
    let hooks: Vec<GithubWebhookResponse> = client
        .get(format!("/repos/{owner}/{repo}/hooks"), None::<&()>)
        .await
        .with_context(|| format!("listing hooks for {owner}/{repo}"))?;

    let desired_url = webhook_config.webhook_url.trim().to_string();
    if let Some(existing) = hooks
        .iter()
        .find(|hook| hook.config.url.as_deref() == Some(desired_url.as_str()))
    {
        return Ok(GithubWebhookRegistration {
            id: existing.id,
            created: false,
        });
    }

    let request = GithubWebhookCreateRequest {
        name: "web".to_string(),
        active: true,
        events: vec!["pull_request".to_string()],
        config: GithubWebhookCreateConfig {
            url: desired_url,
            content_type: "json".to_string(),
            secret: webhook_config.secret.clone(),
            insecure_ssl: "0".to_string(),
        },
    };

    let created: GithubWebhookResponse = client
        .post(format!("/repos/{owner}/{repo}/hooks"), Some(&request))
        .await
        .with_context(|| format!("creating webhook for {owner}/{repo}"))?;

    Ok(GithubWebhookRegistration {
        id: created.id,
        created: true,
    })
}

fn parse_github_remote(remote_url: &str) -> Option<(String, String)> {
    let trimmed = remote_url.trim();
    let without_prefix = if let Some(rest) = trimmed.strip_prefix("https://github.com/") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("http://github.com/") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("git@github.com:") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("ssh://git@github.com/") {
        rest
    } else {
        return None;
    };

    let path = without_prefix.trim_end_matches(".git");
    let mut parts = path.splitn(3, '/');
    let owner = parts.next()?.trim();
    let repo = parts.next()?.trim();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }

    Some((owner.to_string(), repo.to_string()))
}

fn github_client(token: &str) -> Result<Octocrab> {
    Octocrab::builder()
        .personal_token(token.to_string())
        .build()
        .context("building GitHub client")
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::{Method::GET, Method::POST, MockServer};
    use serde_json::json;

    fn test_client(server: &MockServer) -> Octocrab {
        Octocrab::builder()
            .personal_token("token".to_string())
            .base_uri(server.base_url())
            .expect("base uri")
            .build()
            .expect("octocrab client")
    }

    fn test_config() -> ResolvedGithubWebhookConfig {
        ResolvedGithubWebhookConfig {
            secret: "sekret".to_string(),
            webhook_url: "https://metis.example.com/v1/github/webhook".to_string(),
        }
    }

    #[tokio::test]
    async fn register_repo_webhook_reuses_existing_hook() -> Result<()> {
        let server = MockServer::start();
        let list_mock = server.mock(|when, then| {
            when.method(GET).path("/repos/octo/widgets/hooks");
            then.status(200).json_body(json!([{
                "id": 99,
                "config": { "url": "https://metis.example.com/v1/github/webhook" }
            }]));
        });
        let create_mock = server.mock(|when, then| {
            when.method(POST).path("/repos/octo/widgets/hooks");
            then.status(201).json_body(json!({
                "id": 100,
                "config": { "url": "https://metis.example.com/v1/github/webhook" }
            }));
        });

        let client = test_client(&server);
        let registration =
            register_repo_webhook(&client, "octo", "widgets", &test_config()).await?;

        assert_eq!(registration.id, 99);
        assert!(!registration.created);
        list_mock.assert_hits(1);
        create_mock.assert_hits(0);
        Ok(())
    }

    #[tokio::test]
    async fn register_repo_webhook_creates_when_missing() -> Result<()> {
        let server = MockServer::start();
        let list_mock = server.mock(|when, then| {
            when.method(GET).path("/repos/octo/widgets/hooks");
            then.status(200).json_body(json!([]));
        });
        let create_mock = server.mock(|when, then| {
            when.method(POST).path("/repos/octo/widgets/hooks");
            then.status(201).json_body(json!({
                "id": 123,
                "config": { "url": "https://metis.example.com/v1/github/webhook" }
            }));
        });

        let client = test_client(&server);
        let registration =
            register_repo_webhook(&client, "octo", "widgets", &test_config()).await?;

        assert_eq!(registration.id, 123);
        assert!(registration.created);
        list_mock.assert_hits(1);
        create_mock.assert_hits(1);
        Ok(())
    }

    #[test]
    fn parse_github_remote_accepts_https() {
        let (owner, repo) =
            parse_github_remote("https://github.com/example/repo.git").expect("repo");
        assert_eq!(owner, "example");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn parse_github_remote_accepts_ssh() {
        let (owner, repo) = parse_github_remote("git@github.com:example/repo.git").expect("repo");
        assert_eq!(owner, "example");
        assert_eq!(repo, "repo");
    }
}
