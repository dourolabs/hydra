use crate::config::expand_path;
use anyhow::{Context, Result, bail, ensure};
use jsonwebtoken::EncodingKey;
use metis_common::repositories::{GithubAppInstallationConfig, ServiceRepository};
use octocrab::Octocrab;
use octocrab::models::{AppId, InstallationId};
use secrecy::ExposeSecret;
use std::fs;

pub fn build_github_app_client(
    app_id: u64,
    private_key: &str,
    base_uri: Option<&str>,
) -> Result<Octocrab> {
    let key = EncodingKey::from_rsa_pem(private_key.as_bytes())
        .context("invalid GitHub App private key")?;
    let mut builder = Octocrab::builder().app(AppId(app_id), key);
    if let Some(base_uri) = base_uri {
        builder = builder
            .base_uri(base_uri)
            .context("invalid GitHub API base URI")?;
    }
    builder.build().context("building GitHub App client")
}

pub async fn fetch_installation_token_with_client(
    client: &Octocrab,
    installation_id: u64,
) -> Result<String> {
    let token = client
        .installation(InstallationId(installation_id))
        .context("failed to create GitHub App installation client")?
        .installation_token()
        .await
        .context("failed to fetch GitHub App installation token")?;
    Ok(token.expose_secret().to_string())
}

pub async fn fetch_installation_token(config: &GithubAppInstallationConfig) -> Result<String> {
    let private_key = resolve_private_key(config)?;
    let client = build_github_app_client(config.app_id, &private_key, None)?;
    fetch_installation_token_with_client(&client, config.installation_id).await
}

pub async fn resolve_repository_access_token(repo: &ServiceRepository) -> Result<Option<String>> {
    if let Some(github_app) = &repo.github_app {
        let token = fetch_installation_token(github_app).await?;
        return Ok(Some(token));
    }

    let github_token = repo
        .github_token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    Ok(github_token)
}

fn resolve_private_key(config: &GithubAppInstallationConfig) -> Result<String> {
    ensure!(
        config.app_id > 0,
        "github_app.app_id must be a positive integer"
    );
    ensure!(
        config.installation_id > 0,
        "github_app.installation_id must be a positive integer"
    );

    let private_key = config
        .private_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let key_path = config
        .key_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if private_key.is_some() && key_path.is_some() {
        bail!("github_app.private_key and github_app.key_path cannot both be set");
    }

    if let Some(private_key) = private_key {
        return Ok(private_key.to_string());
    }

    let Some(key_path) = key_path else {
        bail!("github_app.private_key or github_app.key_path must be set");
    };

    let path = expand_path(key_path);
    fs::read_to_string(&path).with_context(|| {
        format!(
            "failed to read GitHub App private key from '{}'",
            path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use serde_json::json;

    const TEST_PRIVATE_KEY: &str = "-----BEGIN PRIVATE KEY-----\n\
MIIEugIBADANBgkqhkiG9w0BAQEFAASCBKQwggSgAgEAAoIBAQCWEP+ffdqzFe/A\n\
MIXbhX2gd99fx+vv+cVPnHvLePhMH+SXKVRSxEq7vePypJLLyeDwF1BYCE9zBjcP\n\
spc2f89lD3i+cXNkQoep3a0yivPVvJZHLvLIjlOSX+SWyORa/J57yjVjrXFIMxa2\n\
wBQwXjNH3CbIANA/K1kjlnglEahlsurLGEfv1lk4BC1LsrqzXX4I/G9PABJOc9Ri\n\
dt8SOwjBevZLzT6aEGSa6Cyt2OuTHR7BI6tioe+eBH/HIJ4hUZtLMyRvxBGNB2k0\n\
2BnkM48tArCEv6PEbZ1vSFWOpLEc5fctWhKuBz6pTfHplWDu0/3MbFJhns0as19R\n\
J1Kn7LqZAgMBAAECgf8F3s9GG9Hn+kRaIIFx1JJ5ZVwOXHp+3CakUaCDJopsga3j\n\
sM+oN+9XQNYGa/q1PzuOIufqwek0vG7eZdNciN6au+8wTT5djd4AW9RNytk7UyGS\n\
Qqq+5TQXNppN09xsYV9DsiJaLC/eYszkjQsJWJouIABUGCSDtzxzUKKax74BtzZ3\n\
zX1uZQR6PXpuU3mvcW0Y6caOaLQhAJV5R6zS+qRyFt3/RNHUQQwuSZNle64Gg5hN\n\
coDGLF0F3d2vsa8yysI+ADrFNsrD5EIpQb3dZNp0TktauQwAJuZdvgq7hbdkq7K3\n\
GV3y+z9t8FQnAxjsAkYyInFqvlmeOWngYOY8kWkCgYEAyjLfePunUcCfrra7b4oj\n\
MqC6Gr7zviPK9aQnIVDJPXuda4xijpqP25w3x5GmmFp6auDPH7JoiNLA8tlCgO0s\n\
GIBZimpGhZkq4onSLa1lPfAKVXqCusC4yOK5zZPAYMHadPxJxucIfIbRo1Iw2fgO\n\
urE9CV8ZzitOegz6hZJmW2UCgYEAvf8KFP+OnPKd2YPgpKLHvEzkbJ2m0g1iahsd\n\
USXTEN9hG3zrRT9MRp0sFuYWXuWpYLhQzF1BMN5bAQwHLHAmmxyAQv/W5j/rJCoB\n\
Y2ESZo1+5CHPWxCmmogbxyBoGHFczrGhXdPPPhiLgKSMnVZ3GyYDiyWlU6sl4NgB\n\
6swboSUCgYA1a9l1Gm/rfovx2h+NaZ7BCowA8wBs9QHzgmpAOBrjHpzJxG5ppNZr\n\
PEvUc1vjlswPHtQ6WKWbuKr3voT+kSr8UjTWCBwXwg79iVI5dT1xbtEcImEVvENV\n\
9+kFMos6RR1VmS5Y2cN5Oxl6IAX+ILarhpZMuo6T1QdH4dPypGpcrQKBgEMkdfOl\n\
vEhKlO3hZOnJfLxWkAKyU9m3USgeHOYob8ZuqmqEYsA99j6eHI6bERzIHGtJt4QB\n\
EKCsc4yTK5XQrFP0Zn9G2jLUM8y763GrRE1pg4YrTJPp9nZ10xszoJXCugFxVI1L\n\
5NkU43e6rtaLT9wQOwBZdWtz+BbVPxgyuTDhAoGAa7YBsddTHfAfwZIb/it+8fM6\n\
pr+ova7XfhxC2m0Ko7Ma2KDEit/6UZYyG/sZouc5/uSjaonBY4AgFbxNWb9/73hZ\n\
FZ1BHU7DAzDLPhowUpFPSwOMTMK3pRSWYQmBmXQ1mCSmEt1Z9u7hKTDxHTmTyvOn\n\
FqSFMaLr8qnIav4qMyM=\n\
-----END PRIVATE KEY-----\n";

    #[tokio::test]
    async fn fetch_installation_token_with_client_requests_access_token() -> Result<()> {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST)
                .path("/app/installations/4242/access_tokens");
            then.status(201).json_body(json!({
                "token": "token-123",
                "permissions": {}
            }));
        });

        let client = build_github_app_client(1, TEST_PRIVATE_KEY, Some(&server.base_url()))?;
        let token = fetch_installation_token_with_client(&client, 4242).await?;

        mock.assert();
        assert_eq!(token, "token-123");
        Ok(())
    }
}
