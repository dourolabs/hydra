use crate::{
    client::MetisClientInterface,
    command::output::{render_repository_records, CommandContext, ResolvedOutputFormat},
};
use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use metis_common::{
    constants::MAX_REPOSITORY_SUMMARY_BYTES,
    repositories::{CreateRepositoryRequest, Repository, RepositoryRecord, UpdateRepositoryRequest},
    RepoName,
};
use serde_json::json;
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, Subcommand)]
pub enum ReposCommand {
    /// List configured repositories.
    List,
    /// Create a new repository configuration.
    Create(CreateRepositoryArgs),
    /// Update an existing repository configuration.
    Update(UpdateRepositoryArgs),
    /// Show repository details, including the stored content summary.
    Details(ReposDetailsArgs),
}

#[derive(Debug, Clone, Args)]
pub struct CreateRepositoryArgs {
    /// Repository name in the form org/repo.
    #[arg(value_name = "NAME")]
    pub name: RepoName,

    /// Remote git URL reachable by metis workers.
    #[arg(value_name = "REMOTE_URL")]
    pub remote_url: String,

    /// Default branch to use when not explicitly provided.
    #[arg(
        long = "default-branch",
        value_name = "BRANCH",
        conflicts_with = "clear_default_branch"
    )]
    pub default_branch: Option<String>,

    /// Clear the configured default branch.
    #[arg(long = "clear-default-branch")]
    pub clear_default_branch: bool,

    /// Default container image for jobs from this repository.
    #[arg(
        long = "default-image",
        value_name = "IMAGE",
        conflicts_with = "clear_default_image"
    )]
    pub default_image: Option<String>,

    /// Clear the configured default image.
    #[arg(long = "clear-default-image")]
    pub clear_default_image: bool,
}

#[derive(Debug, Clone, Args)]
pub struct UpdateRepositoryArgs {
    /// Repository name in the form org/repo.
    #[arg(value_name = "NAME")]
    pub name: RepoName,

    /// Remote git URL reachable by metis workers.
    #[arg(long = "remote-url", value_name = "REMOTE_URL")]
    pub remote_url: Option<String>,

    /// Default branch to use when not explicitly provided.
    #[arg(
        long = "default-branch",
        value_name = "BRANCH",
        conflicts_with = "clear_default_branch"
    )]
    pub default_branch: Option<String>,

    /// Clear the configured default branch.
    #[arg(long = "clear-default-branch")]
    pub clear_default_branch: bool,

    /// Default container image for jobs from this repository.
    #[arg(
        long = "default-image",
        value_name = "IMAGE",
        conflicts_with = "clear_default_image"
    )]
    pub default_image: Option<String>,

    /// Clear the configured default image.
    #[arg(long = "clear-default-image")]
    pub clear_default_image: bool,

    /// Markdown file to use for the content summary; pass '-' to read from stdin.
    #[arg(
        long = "summary-file",
        value_name = "FILE",
        conflicts_with = "clear_summary"
    )]
    pub summary_file: Option<PathBuf>,

    /// Clear the stored summary.
    #[arg(long = "clear-summary")]
    pub clear_summary: bool,
}

#[derive(Debug, Clone, Args)]
pub struct ReposDetailsArgs {
    /// Repository name in the form org/repo.
    #[arg(value_name = "NAME")]
    pub name: RepoName,
}

pub async fn run(
    client: &dyn MetisClientInterface,
    command: ReposCommand,
    context: &CommandContext,
) -> Result<()> {
    let mut stdout = io::stdout().lock();
    match command {
        ReposCommand::List => {
            let repositories = fetch_repositories(client).await?;
            render_repository_records(context.output_format, &repositories, &mut stdout)?;
        }
        ReposCommand::Create(args) => {
            let repository = create_repository(client, args).await?;
            render_repository_records(context.output_format, &[repository], &mut stdout)?;
        }
        ReposCommand::Update(args) => {
            let repository = update_repository(client, args).await?;
            render_repository_records(context.output_format, &[repository], &mut stdout)?;
        }
        ReposCommand::Details(args) => {
            show_details(client, args, context, &mut stdout).await?;
        }
    }

    Ok(())
}

async fn fetch_repositories(client: &dyn MetisClientInterface) -> Result<Vec<RepositoryRecord>> {
    let response = client
        .list_repositories()
        .await
        .context("failed to list repositories")?;
    Ok(response.repositories)
}

async fn fetch_repository_by_name(
    client: &dyn MetisClientInterface,
    name: &RepoName,
) -> Result<RepositoryRecord> {
    let repositories = fetch_repositories(client).await?;
    repositories
        .into_iter()
        .find(|repository| &repository.name == name)
        .with_context(|| format!("repository '{name}' not found"))
}

async fn create_repository(
    client: &dyn MetisClientInterface,
    args: CreateRepositoryArgs,
) -> Result<RepositoryRecord> {
    let request = build_create_request(&args)?;
    let response = client
        .create_repository(&request)
        .await
        .context("failed to create repository")?;
    Ok(response.repository)
}

async fn update_repository(
    client: &dyn MetisClientInterface,
    args: UpdateRepositoryArgs,
) -> Result<RepositoryRecord> {
    let (repo_name, request) = build_update_request(client, &args).await?;
    let response = client
        .update_repository(&repo_name, &request)
        .await
        .context("failed to update repository")?;
    Ok(response.repository)
}

async fn show_details(
    client: &dyn MetisClientInterface,
    args: ReposDetailsArgs,
    context: &CommandContext,
    writer: &mut dyn Write,
) -> Result<()> {
    let repo = fetch_repository_by_name(client, &args.name)
        .await
        .context("failed to load repository details")?;
    let summary = repo.repository.content_summary.clone();

    match context.output_format {
        ResolvedOutputFormat::Pretty => {
            if let Some(markdown) = summary {
                writer.write_all(markdown.as_bytes())?;
                if !markdown.ends_with('\n') {
                    writer.write_all(b"\n")?;
                }
            } else {
                writeln!(
                    writer,
                    "Repository '{}' does not have a content summary.",
                    args.name
                )?;
            }
        }
        ResolvedOutputFormat::Jsonl => {
            serde_json::to_writer(
                &mut *writer,
                &json!({
                    "name": args.name,
                    "content_summary": summary,
                }),
            )?;
            writer.write_all(b"\n")?;
        }
    }

    writer.flush()?;
    Ok(())
}

fn build_create_request(args: &CreateRepositoryArgs) -> Result<CreateRepositoryRequest> {
    Ok(CreateRepositoryRequest::new(
        args.name.clone(),
        build_repository_config(
            parse_required(&args.remote_url, "remote URL")?,
            &args.default_branch,
            args.clear_default_branch,
            &args.default_image,
            args.clear_default_image,
        )?,
    ))
}

async fn build_update_request(
    client: &dyn MetisClientInterface,
    args: &UpdateRepositoryArgs,
) -> Result<(RepoName, UpdateRepositoryRequest)> {
    let existing = fetch_repository_by_name(client, &args.name)
        .await
        .context("failed to load repository config")?;
    let current = existing.repository;

    let remote_url = resolve_remote_url_arg(&args.remote_url, &current.remote_url)?;
    let default_branch = resolve_optional_with_current(
        &args.default_branch,
        args.clear_default_branch,
        &current.default_branch,
        "default branch",
        "--clear-default-branch",
    )?;
    let default_image = resolve_optional_with_current(
        &args.default_image,
        args.clear_default_image,
        &current.default_image,
        "default image",
        "--clear-default-image",
    )?;
    let content_summary = apply_summary_override(
        resolve_summary_override(args)?,
        current.content_summary.clone(),
    );

    Ok((
        args.name.clone(),
        UpdateRepositoryRequest::new(Repository::new(
            remote_url,
            default_branch,
            default_image,
            content_summary,
        )),
    ))
}

fn resolve_remote_url_arg(remote_url: &Option<String>, current: &str) -> Result<String> {
    if let Some(url) = remote_url {
        return parse_required(url, "remote URL");
    }

    Ok(current.to_string())
}

fn resolve_optional_with_current(
    value: &Option<String>,
    clear_flag: bool,
    current: &Option<String>,
    field: &str,
    clear_arg: &str,
) -> Result<Option<String>> {
    if clear_flag {
        return Ok(None);
    }

    match value {
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                bail!("{field} must not be empty (use {clear_arg} to clear it)");
            }
            Ok(Some(trimmed.to_string()))
        }
        None => Ok(current.clone()),
    }
}

fn build_repository_config(
    remote_url: String,
    default_branch: &Option<String>,
    clear_default_branch: bool,
    default_image: &Option<String>,
    clear_default_image: bool,
) -> Result<Repository> {
    Ok(Repository::new(
        remote_url,
        parse_optional(
            default_branch,
            clear_default_branch,
            "default branch",
            "--clear-default-branch",
        )?,
        parse_optional(
            default_image,
            clear_default_image,
            "default image",
            "--clear-default-image",
        )?,
        None,
    ))
}

#[derive(Debug)]
enum SummaryOverride {
    Unchanged,
    Clear,
    Set(String),
}

fn resolve_summary_override(args: &UpdateRepositoryArgs) -> Result<SummaryOverride> {
    if args.clear_summary {
        return Ok(SummaryOverride::Clear);
    }

    if let Some(path) = args.summary_file.as_deref() {
        let contents = read_summary_from_source(path)?;
        validate_summary_contents(&contents)?;
        return Ok(SummaryOverride::Set(contents));
    }

    Ok(SummaryOverride::Unchanged)
}

fn apply_summary_override(
    summary_override: SummaryOverride,
    current: Option<String>,
) -> Option<String> {
    match summary_override {
        SummaryOverride::Unchanged => current,
        SummaryOverride::Clear => None,
        SummaryOverride::Set(value) => Some(value),
    }
}

fn read_summary_from_source(path: &Path) -> Result<String> {
    if path == Path::new("-") {
        let mut buffer = String::new();
        io::stdin()
            .read_to_string(&mut buffer)
            .context("failed to read summary from stdin")?;
        return Ok(buffer);
    }

    fs::read_to_string(path)
        .with_context(|| format!("failed to read summary file '{}'", path.display()))
}

fn validate_summary_contents(contents: &str) -> Result<()> {
    if contents.trim().is_empty() {
        bail!("content summary must not be empty");
    }

    if contents.len() > MAX_REPOSITORY_SUMMARY_BYTES {
        bail!("content summary must be at most {MAX_REPOSITORY_SUMMARY_BYTES} bytes");
    }

    Ok(())
}

fn parse_required(value: &str, field: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("{field} must not be empty");
    }

    Ok(trimmed.to_string())
}

fn parse_optional(
    value: &Option<String>,
    clear_flag: bool,
    field: &str,
    clear_arg: &str,
) -> Result<Option<String>> {
    if clear_flag {
        return Ok(None);
    }

    match value {
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                bail!("{field} must not be empty (use {clear_arg} to clear it)");
            }
            Ok(Some(trimmed.to_string()))
        }
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        client::MetisClient,
        command::output::{render_repository_records, CommandContext, ResolvedOutputFormat},
    };
    use httpmock::prelude::*;
    use metis_common::repositories::{
        ListRepositoriesResponse, Repository, RepositoryRecord, UpsertRepositoryResponse,
    };
    use reqwest::Client as HttpClient;
    use serde_json::json;
    use std::str::FromStr;
    use tempfile::NamedTempFile;

    const TEST_METIS_TOKEN: &str = "test-metis-token";

    fn sample_create_args() -> CreateRepositoryArgs {
        CreateRepositoryArgs {
            name: RepoName::from_str("dourolabs/metis").unwrap(),
            remote_url: "https://example.com/metis.git".to_string(),
            default_branch: Some("main".to_string()),
            clear_default_branch: false,
            default_image: Some("ghcr.io/dourolabs/metis:latest".to_string()),
            clear_default_image: false,
        }
    }

    fn sample_update_args() -> UpdateRepositoryArgs {
        UpdateRepositoryArgs {
            name: RepoName::from_str("dourolabs/metis").unwrap(),
            remote_url: Some("https://example.com/metis.git".to_string()),
            default_branch: Some("main".to_string()),
            clear_default_branch: false,
            default_image: Some("ghcr.io/dourolabs/metis:latest".to_string()),
            clear_default_image: false,
            summary_file: None,
            clear_summary: false,
        }
    }

    fn sample_repository_info(name: &RepoName) -> RepositoryRecord {
        RepositoryRecord::new(
            name.clone(),
            Repository::new(
                "https://example.com/metis.git".to_string(),
                Some("main".to_string()),
                Some("ghcr.io/dourolabs/metis:latest".to_string()),
                None,
            ),
        )
    }

    fn mock_client(server: &MockServer) -> MetisClient {
        MetisClient::with_http_client(server.base_url(), TEST_METIS_TOKEN, HttpClient::new())
            .expect("mock client creation should not fail")
    }

    #[tokio::test]
    async fn list_repositories_prints_all_fields() {
        let repo_name = RepoName::from_str("dourolabs/metis").unwrap();
        let repositories = ListRepositoriesResponse::new(vec![
            sample_repository_info(&repo_name),
            RepositoryRecord::new(
                RepoName::from_str("dourolabs/api").unwrap(),
                Repository::new(
                    "git@github.com:dourolabs/api.git".to_string(),
                    None,
                    None,
                    None,
                ),
            ),
        ]);
        let server = MockServer::start();
        let list_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/repositories");
            then.status(200).json_body_obj(&repositories);
        });
        let client = mock_client(&server);

        let repositories = fetch_repositories(&client).await.unwrap();
        let mut output = Vec::new();
        render_repository_records(ResolvedOutputFormat::Pretty, &repositories, &mut output)
            .unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(output.contains("dourolabs/metis"));
        assert!(output.contains("remote_url: https://example.com/metis.git"));
        assert!(output.contains("default_branch: main"));
        assert!(output.contains("default_image: ghcr.io/dourolabs/metis:latest"));
        assert!(output.contains("content_summary: <none>"));
        assert!(output.contains("dourolabs/api"));

        list_mock.assert();
    }

    #[tokio::test]
    async fn list_repositories_reports_client_error() {
        let server = MockServer::start();
        let list_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/repositories");
            then.status(500);
        });
        let client = mock_client(&server);

        let error = fetch_repositories(&client).await.unwrap_err();
        assert!(
            error.to_string().contains("failed to list repositories"),
            "error should include context: {error:?}"
        );

        list_mock.assert();
    }

    #[tokio::test]
    async fn create_repository_sends_request_and_prints_result() {
        let args = sample_create_args();
        let server = MockServer::start();
        let repository = sample_repository_info(&args.name);
        let create_mock = server.mock(|when, then| {
            when.method(POST).path("/v1/repositories").json_body(json!({
                "name": "dourolabs/metis",
                "remote_url": "https://example.com/metis.git",
                "default_branch": "main",
                "default_image": "ghcr.io/dourolabs/metis:latest",
                "content_summary": null
            }));
            then.status(200)
                .json_body_obj(&UpsertRepositoryResponse::new(repository.clone()));
        });
        let client = mock_client(&server);

        let repository = create_repository(&client, args.clone()).await.unwrap();

        let mut output = Vec::new();
        render_repository_records(ResolvedOutputFormat::Pretty, &[repository], &mut output)
            .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("dourolabs/metis"));

        create_mock.assert();
    }

    #[tokio::test]
    async fn create_repository_rejects_empty_remote_url() {
        let server = MockServer::start();
        let client = mock_client(&server);
        let mut args = sample_create_args();
        args.remote_url = "   ".to_string();

        let error = create_repository(&client, args).await.unwrap_err();
        assert!(
            error.to_string().contains("remote URL must not be empty"),
            "error should mention missing remote URL: {error:?}"
        );
    }

    #[tokio::test]
    async fn update_repository_sends_request_and_allows_clearing_fields() {
        let mut args = sample_update_args();
        args.clear_default_branch = true;
        args.default_branch = None;
        args.default_image = Some("ghcr.io/dourolabs/metis:stable".to_string());
        let server = MockServer::start();
        let list_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/repositories");
            then.status(200)
                .json_body_obj(&ListRepositoriesResponse::new(vec![
                    sample_repository_info(&args.name),
                ]));
        });
        let update_mock = server.mock(|when, then| {
            when.method(PUT)
                .path("/v1/repositories/dourolabs/metis")
                .json_body(json!({
                    "remote_url": "https://example.com/metis.git",
                    "default_branch": null,
                    "default_image": "ghcr.io/dourolabs/metis:stable",
                    "content_summary": null
                }));
            then.status(200)
                .json_body_obj(&UpsertRepositoryResponse::new(RepositoryRecord::new(
                    args.name.clone(),
                    Repository::new(
                        args.remote_url.clone().unwrap(),
                        None,
                        args.default_image.clone(),
                        None,
                    ),
                )));
        });
        let client = mock_client(&server);

        let repository = update_repository(&client, args.clone()).await.unwrap();

        let mut output = Vec::new();
        render_repository_records(ResolvedOutputFormat::Pretty, &[repository], &mut output)
            .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("default_branch: <none>"));

        update_mock.assert();
        list_mock.assert();
    }

    #[tokio::test]
    async fn update_repository_uses_remote_url_from_listing() {
        let mut args = sample_update_args();
        args.remote_url = None;
        args.default_branch = None;
        args.default_image = Some("ghcr.io/dourolabs/metis:stable".to_string());
        let server = MockServer::start();
        let list_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/repositories");
            then.status(200)
                .json_body_obj(&ListRepositoriesResponse::new(vec![
                    sample_repository_info(&args.name),
                ]));
        });
        let update_mock = server.mock(|when, then| {
            when.method(PUT)
                .path("/v1/repositories/dourolabs/metis")
                .json_body(json!({
                    "remote_url": "https://example.com/metis.git",
                    "default_branch": "main",
                    "default_image": "ghcr.io/dourolabs/metis:stable",
                    "content_summary": null
                }));
            then.status(200)
                .json_body_obj(&UpsertRepositoryResponse::new(RepositoryRecord::new(
                    args.name.clone(),
                    Repository::new(
                        "https://example.com/metis.git".to_string(),
                        Some("main".to_string()),
                        args.default_image.clone(),
                        None,
                    ),
                )));
        });
        let client = mock_client(&server);

        let repository = update_repository(&client, args).await.unwrap();

        assert_eq!(
            repository.repository.remote_url,
            "https://example.com/metis.git"
        );
        list_mock.assert();
        update_mock.assert();
    }

    #[tokio::test]
    async fn update_repository_reports_client_error() {
        let server = MockServer::start();
        let args = sample_update_args();
        let list_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/repositories");
            then.status(200)
                .json_body_obj(&ListRepositoriesResponse::new(vec![
                    sample_repository_info(&args.name),
                ]));
        });
        let update_mock = server.mock(|when, then| {
            when.method(PUT)
                .path("/v1/repositories/dourolabs/metis")
                .json_body(json!({
                    "remote_url": "https://example.com/metis.git",
                    "default_branch": "main",
                    "default_image": "ghcr.io/dourolabs/metis:latest",
                    "content_summary": null
                }));
            then.status(404);
        });
        let client = mock_client(&server);

        let error = update_repository(&client, args).await.unwrap_err();
        assert!(
            error.to_string().contains("failed to update repository"),
            "error should include context: {error:?}"
        );

        update_mock.assert();
        list_mock.assert();
    }

    #[tokio::test]
    async fn details_prints_markdown_in_pretty_mode() {
        let repo_name = RepoName::from_str("dourolabs/metis").unwrap();
        let repositories = ListRepositoriesResponse::new(vec![RepositoryRecord::new(
            repo_name.clone(),
            Repository::new(
                "https://example.com/metis.git".to_string(),
                Some("main".to_string()),
                None,
                Some("## Summary\nLine".to_string()),
            ),
        )]);
        let server = MockServer::start();
        let list_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/repositories");
            then.status(200).json_body_obj(&repositories);
        });
        let client = mock_client(&server);
        let args = ReposDetailsArgs {
            name: repo_name.clone(),
        };
        let context = CommandContext::new(ResolvedOutputFormat::Pretty);
        let mut output = Vec::new();

        show_details(&client, args, &context, &mut output)
            .await
            .unwrap();

        list_mock.assert();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "## Summary\nLine\n".to_string()
        );
    }

    #[tokio::test]
    async fn details_reports_missing_summary() {
        let repo_name = RepoName::from_str("dourolabs/metis").unwrap();
        let repositories = ListRepositoriesResponse::new(vec![RepositoryRecord::new(
            repo_name.clone(),
            Repository::new(
                "https://example.com/metis.git".to_string(),
                None,
                None,
                None,
            ),
        )]);
        let server = MockServer::start();
        let list_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/repositories");
            then.status(200).json_body_obj(&repositories);
        });
        let client = mock_client(&server);
        let args = ReposDetailsArgs {
            name: repo_name.clone(),
        };
        let context = CommandContext::new(ResolvedOutputFormat::Pretty);
        let mut output = Vec::new();

        show_details(&client, args, &context, &mut output)
            .await
            .unwrap();

        list_mock.assert();
        let rendered = String::from_utf8(output).unwrap();
        assert!(rendered.contains("does not have a content summary"));
    }

    #[tokio::test]
    async fn details_supports_jsonl_output() {
        let repo_name = RepoName::from_str("dourolabs/metis").unwrap();
        let repositories = ListRepositoriesResponse::new(vec![RepositoryRecord::new(
            repo_name.clone(),
            Repository::new(
                "https://example.com/metis.git".to_string(),
                None,
                None,
                Some("## Summary".to_string()),
            ),
        )]);
        let server = MockServer::start();
        let list_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/repositories");
            then.status(200).json_body_obj(&repositories);
        });
        let client = mock_client(&server);
        let args = ReposDetailsArgs { name: repo_name };
        let context = CommandContext::new(ResolvedOutputFormat::Jsonl);
        let mut output = Vec::new();

        show_details(&client, args, &context, &mut output)
            .await
            .unwrap();

        list_mock.assert();
        let rendered = String::from_utf8(output).unwrap();
        assert!(
            rendered.contains("\"content_summary\":\"## Summary\""),
            "{rendered}"
        );
    }

    #[tokio::test]
    async fn update_repository_reads_summary_from_file() {
        let mut args = sample_update_args();
        let server = MockServer::start();
        let list_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/repositories");
            then.status(200)
                .json_body_obj(&ListRepositoriesResponse::new(vec![
                    sample_repository_info(&args.name),
                ]));
        });
        let mut file = NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut file, b"## CLI Summary").unwrap();
        args.summary_file = Some(file.path().to_path_buf());

        let update_mock = server.mock(|when, then| {
            when.method(PUT)
                .path("/v1/repositories/dourolabs/metis")
                .json_body(json!({
                    "remote_url": "https://example.com/metis.git",
                    "default_branch": "main",
                    "default_image": "ghcr.io/dourolabs/metis:latest",
                    "content_summary": "## CLI Summary"
                }));
            then.status(200)
                .json_body_obj(&UpsertRepositoryResponse::new(sample_repository_info(
                    &args.name,
                )));
        });
        let client = mock_client(&server);

        update_repository(&client, args).await.unwrap();

        list_mock.assert();
        update_mock.assert();
    }

    #[tokio::test]
    async fn update_repository_clears_summary_when_flag_set() {
        let mut args = sample_update_args();
        args.summary_file = None;
        args.clear_summary = true;
        let server = MockServer::start();
        let existing = RepositoryRecord::new(
            args.name.clone(),
            Repository::new(
                "https://example.com/metis.git".to_string(),
                Some("main".to_string()),
                Some("ghcr.io/dourolabs/metis:latest".to_string()),
                Some("## Old summary".to_string()),
            ),
        );
        let list_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/repositories");
            then.status(200)
                .json_body_obj(&ListRepositoriesResponse::new(vec![existing.clone()]));
        });
        let update_mock = server.mock(|when, then| {
            when.method(PUT)
                .path("/v1/repositories/dourolabs/metis")
                .json_body(json!({
                    "remote_url": "https://example.com/metis.git",
                    "default_branch": "main",
                    "default_image": "ghcr.io/dourolabs/metis:latest",
                    "content_summary": null
                }));
            then.status(200)
                .json_body_obj(&UpsertRepositoryResponse::new(existing.clone()));
        });
        let client = mock_client(&server);

        update_repository(&client, args).await.unwrap();

        list_mock.assert();
        update_mock.assert();
    }
}
