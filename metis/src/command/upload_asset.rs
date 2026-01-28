use std::{
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, bail, Context, Result};
use clap::Args;
use git2::Repository;
use metis_common::{
    constants::{ENV_GITHUB_REPOSITORY, ENV_PR_NUMBER},
    RepoName,
};
use reqwest::{header, Client as HttpClient, Url};
use serde::{Deserialize, Serialize};

use crate::{
    client::MetisClientInterface,
    command::output::{CommandContext, ResolvedOutputFormat},
    git,
};

#[derive(Debug, Args)]
pub struct UploadAssetArgs {
    /// Local file path to upload.
    #[arg(value_name = "FILE")]
    pub file: PathBuf,

    /// Repository name in the form org/repo (defaults to origin remote).
    #[arg(long = "repo", value_name = "REPO", env = ENV_GITHUB_REPOSITORY)]
    pub repo: Option<RepoName>,

    /// Pull request number to attach the asset to.
    #[arg(long = "pr-number", value_name = "PR_NUMBER", env = ENV_PR_NUMBER)]
    pub pr_number: Option<u64>,

    /// Optional filename override for the uploaded asset.
    #[arg(long = "name", value_name = "NAME")]
    pub name: Option<String>,

    /// Override the content type (defaults based on file extension).
    #[arg(long = "content-type", value_name = "CONTENT_TYPE")]
    pub content_type: Option<String>,
}

#[derive(Debug, Serialize)]
struct UploadAssetOutput {
    url: String,
    markdown: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UploadAssetResponse {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    markdown: Option<String>,
    #[serde(rename = "html_url", default)]
    html_url: Option<String>,
}

pub async fn run(
    client: &dyn MetisClientInterface,
    args: UploadAssetArgs,
    context: &CommandContext,
) -> Result<()> {
    let repo_root = git::repository_root(None)?;
    let repo = resolve_repo_name(args.repo, &repo_root)?;
    let pr_number = args
        .pr_number
        .ok_or_else(|| anyhow!("PR number is required; use --pr-number or set PR_NUMBER"))?;
    let token = resolve_github_token(client).await?;
    let name = resolve_asset_name(&args.file, args.name.as_deref())?;
    let content_type = args
        .content_type
        .unwrap_or_else(|| guess_content_type(&args.file).to_string());
    let url = build_upload_url(&repo, pr_number, &name)?;

    let bytes = tokio::fs::read(&args.file)
        .await
        .with_context(|| format!("failed to read {}", args.file.display()))?;
    let response = upload_asset(&token, url, content_type, bytes).await?;
    let asset_url = resolve_asset_url(&response)?
        .ok_or_else(|| anyhow!("GitHub upload response did not include a usable asset URL"))?;

    write_output(
        context.output_format,
        &UploadAssetOutput {
            url: asset_url,
            markdown: response.markdown,
        },
    )?;

    Ok(())
}

async fn resolve_github_token(client: &dyn MetisClientInterface) -> Result<String> {
    client
        .get_github_token()
        .await
        .context("failed to fetch GitHub token from Metis")
}

fn resolve_repo_name(repo: Option<RepoName>, repo_root: &Path) -> Result<RepoName> {
    if let Some(repo) = repo {
        return Ok(repo);
    }

    github_repo_from_origin(repo_root)
}

fn resolve_asset_name(path: &Path, override_name: Option<&str>) -> Result<String> {
    if let Some(name) = override_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(name.to_string());
    }

    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_string())
        .ok_or_else(|| anyhow!("failed to infer asset name from {}", path.display()))
}

fn build_upload_url(repo: &RepoName, pr_number: u64, name: &str) -> Result<Url> {
    let base = format!(
        "https://uploads.github.com/repos/{}/{}/issues/{}/comments",
        repo.organization, repo.repo, pr_number
    );
    let mut url = Url::parse(&base).context("failed to build GitHub uploads URL")?;
    url.query_pairs_mut().append_pair("name", name);
    Ok(url)
}

async fn upload_asset(
    token: &str,
    url: Url,
    content_type: String,
    body: Vec<u8>,
) -> Result<UploadAssetResponse> {
    let client = HttpClient::new();
    let response = client
        .post(url)
        .header(header::AUTHORIZATION, format!("token {}", token.trim()))
        .header(header::ACCEPT, "application/vnd.github+json")
        .header(header::USER_AGENT, "metis-cli")
        .header(header::CONTENT_TYPE, content_type)
        .body(body)
        .send()
        .await
        .context("failed to send upload request to GitHub")?;

    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<failed to read response body>".to_string());
        bail!("GitHub upload failed ({status}): {body}");
    }

    response
        .json::<UploadAssetResponse>()
        .await
        .context("failed to parse GitHub upload response")
}

fn resolve_asset_url(response: &UploadAssetResponse) -> Result<Option<String>> {
    if let Some(markdown) = response.markdown.as_deref() {
        if let Some(url) = asset_url_from_markdown(markdown) {
            return Ok(Some(url));
        }
    }

    if let Some(html_url) = response.html_url.as_deref() {
        return Ok(Some(html_url.to_string()));
    }

    Ok(response.url.clone())
}

fn asset_url_from_markdown(markdown: &str) -> Option<String> {
    let start = markdown.find("](")? + 2;
    let end = markdown[start..].find(')')?;
    Some(markdown[start..start + end].to_string())
}

fn github_repo_from_origin(repo_root: &Path) -> Result<RepoName> {
    let repo = Repository::discover(repo_root)
        .context("failed to open git repository to determine GitHub repo")?;
    let remote = repo
        .find_remote("origin")
        .context("failed to find 'origin' remote")?;
    let url = remote
        .url()
        .ok_or_else(|| anyhow!("origin remote has no URL"))?;
    let (owner, repo) = parse_github_remote_url(url)
        .ok_or_else(|| anyhow!("failed to parse GitHub repo from origin URL '{url}'"))?;
    RepoName::new(owner, repo).map_err(|err| anyhow!(err))
}

fn parse_github_remote_url(url: &str) -> Option<(String, String)> {
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

fn guess_content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        _ => "application/octet-stream",
    }
}

fn write_output(format: ResolvedOutputFormat, output: &UploadAssetOutput) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    match format {
        ResolvedOutputFormat::Pretty => {
            writeln!(stdout, "{}", output.url)?;
        }
        ResolvedOutputFormat::Jsonl => {
            serde_json::to_writer(&mut stdout, output)?;
            writeln!(stdout)?;
        }
    }
    stdout.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Repository;
    use tempfile::tempdir;

    #[test]
    fn asset_url_from_markdown_extracts_url() {
        let url = asset_url_from_markdown("![image](https://example.com/assets/123)");
        assert_eq!(url, Some("https://example.com/assets/123".to_string()));
    }

    #[test]
    fn asset_url_from_markdown_returns_none_on_invalid() {
        assert!(asset_url_from_markdown("no link").is_none());
    }

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
    fn github_repo_from_origin_reads_origin_remote() -> Result<()> {
        let temp = tempdir().expect("tempdir");
        let repo = Repository::init(temp.path()).expect("init repo");
        repo.remote("origin", "git@github.com:octo/example.git")
            .expect("add remote");

        let repo_name = github_repo_from_origin(temp.path())?;
        assert_eq!(repo_name.as_str(), "octo/example");
        Ok(())
    }

    #[test]
    fn build_upload_url_encodes_name() -> Result<()> {
        let repo = RepoName::new("octo", "example").expect("repo");
        let url = build_upload_url(&repo, 42, "my image.png")?;
        assert_eq!(
            url.as_str(),
            "https://uploads.github.com/repos/octo/example/issues/42/comments?name=my+image.png"
        );
        Ok(())
    }

    #[test]
    fn guess_content_type_defaults_to_octet_stream() {
        assert_eq!(
            guess_content_type(Path::new("unknown.bin")),
            "application/octet-stream"
        );
    }

    #[test]
    fn guess_content_type_handles_png() {
        assert_eq!(guess_content_type(Path::new("image.png")), "image/png");
    }
}
