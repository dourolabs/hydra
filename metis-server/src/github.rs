use anyhow::Result;
use octocrab::Octocrab;
use secrecy::ExposeSecret;
use tracing::warn;

pub struct GithubInstallation {
    pub client: Octocrab,
    pub token: String,
}

pub fn parse_github_remote_url(url: &str) -> Option<(String, String)> {
    let trimmed = url.trim();
    let without_prefix = trimmed
        .strip_prefix("https://github.com/")
        .or_else(|| trimmed.strip_prefix("http://github.com/"))
        .or_else(|| trimmed.strip_prefix("ssh://git@github.com/"))
        .or_else(|| trimmed.strip_prefix("git@github.com:"))?;
    let mut segments = without_prefix.split('/');
    let owner = segments.next()?;
    let repo = segments.next()?;
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

pub async fn select_github_installation(
    app_client: Option<&Octocrab>,
    owner: &str,
    repo: &str,
) -> Result<Option<GithubInstallation>> {
    let Some(app_client) = app_client else {
        return Ok(None);
    };

    let installation = match app_client
        .apps()
        .get_repository_installation(owner, repo)
        .await
    {
        Ok(installation) => installation,
        Err(err) => {
            warn!(
                owner = %owner,
                repo = %repo,
                error = %err,
                "failed to lookup GitHub App installation"
            );
            return Ok(None);
        }
    };

    let (installation_client, token) =
        match app_client.installation_and_token(installation.id).await {
            Ok(result) => result,
            Err(err) => {
                warn!(
                    owner = %owner,
                    repo = %repo,
                    installation_id = %installation.id,
                    error = %err,
                    "failed to fetch GitHub App installation token"
                );
                return Ok(None);
            }
        };

    Ok(Some(GithubInstallation {
        client: installation_client,
        token: token.expose_secret().to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::parse_github_remote_url;

    #[test]
    fn parse_github_remote_url_handles_https_and_ssh() {
        assert_eq!(
            parse_github_remote_url("https://github.com/dourolabs/metis.git"),
            Some(("dourolabs".to_string(), "metis".to_string()))
        );
        assert_eq!(
            parse_github_remote_url("git@github.com:dourolabs/metis.git"),
            Some(("dourolabs".to_string(), "metis".to_string()))
        );
        assert_eq!(
            parse_github_remote_url("ssh://git@github.com/dourolabs/metis"),
            Some(("dourolabs".to_string(), "metis".to_string()))
        );
    }

    #[test]
    fn parse_github_remote_url_rejects_non_github() {
        assert_eq!(
            parse_github_remote_url("https://example.com/dourolabs/metis.git"),
            None
        );
    }
}
