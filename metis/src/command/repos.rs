use crate::{
    client::MetisClientInterface,
    command::output::{render_repository_records, CommandContext, ResolvedOutputFormat},
};
use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use metis_common::{
    constants::MAX_REPOSITORY_SUMMARY_BYTES,
    repositories::{
        CreateRepositoryRequest, Repository, RepositoryRecord, SetRepositorySummaryRequest,
        UpdateRepositoryRequest,
    },
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
    /// Show or modify repository content summaries.
    Summary {
        #[command(subcommand)]
        command: ReposSummaryCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum ReposSummaryCommand {
    /// Print the stored content summary for a repository.
    Show(ReposSummaryShowArgs),
    /// Replace or clear the stored content summary.
    Set(ReposSummarySetArgs),
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
}

#[derive(Debug, Clone, Args)]
pub struct ReposSummaryShowArgs {
    /// Repository name in the form org/repo.
    #[arg(value_name = "NAME")]
    pub name: RepoName,
}

#[derive(Debug, Clone, Args)]
pub struct ReposSummarySetArgs {
    /// Repository name in the form org/repo.
    #[arg(value_name = "NAME")]
    pub name: RepoName,

    /// Markdown file to read; pass '-' to read from stdin.
    #[arg(long = "file", value_name = "FILE", conflicts_with = "clear")]
    pub file: Option<PathBuf>,

    /// Clear the stored summary.
    #[arg(long = "clear")]
    pub clear: bool,
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
        ReposCommand::Summary { command } => match command {
            ReposSummaryCommand::Show(args) => {
                summary_show(client, args, context, &mut stdout).await?;
            }
            ReposSummaryCommand::Set(args) => {
                summary_set(client, args, context, &mut stdout).await?;
            }
        },
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

async fn summary_show(
    client: &dyn MetisClientInterface,
    args: ReposSummaryShowArgs,
    context: &CommandContext,
    writer: &mut dyn Write,
) -> Result<()> {
    let repo_name = args.name;
    let response = client
        .get_repository(&repo_name)
        .await
        .context("failed to fetch repository summary")?;
    let summary = response.repository.repository.content_summary.clone();

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
                    "Repository '{repo_name}' does not have a content summary."
                )?;
            }
        }
        ResolvedOutputFormat::Jsonl => {
            serde_json::to_writer(
                &mut *writer,
                &json!({
                    "name": repo_name,
                    "content_summary": summary,
                }),
            )?;
            writer.write_all(b"\n")?;
        }
    }

    writer.flush()?;
    Ok(())
}

async fn summary_set(
    client: &dyn MetisClientInterface,
    args: ReposSummarySetArgs,
    context: &CommandContext,
    writer: &mut dyn Write,
) -> Result<()> {
    let repo_name = args.name.clone();
    let content_summary = resolve_summary_input(&args)?;
    let request = SetRepositorySummaryRequest::new(content_summary.clone());
    let response = client
        .set_repository_summary(&args.name, &request)
        .await
        .context("failed to update repository summary")?;

    match context.output_format {
        ResolvedOutputFormat::Pretty => {
            if content_summary.is_some() {
                writeln!(writer, "Updated content summary for {repo_name}.")?;
            } else {
                writeln!(writer, "Cleared content summary for {repo_name}.")?;
            }
        }
        ResolvedOutputFormat::Jsonl => {
            serde_json::to_writer(
                &mut *writer,
                &json!({
                    "name": response.repository.name,
                    "content_summary": response.repository.repository.content_summary,
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
    let remote_url = resolve_remote_url(client, args).await?;
    Ok((
        args.name.clone(),
        UpdateRepositoryRequest::new(build_repository_config(
            remote_url,
            &args.default_branch,
            args.clear_default_branch,
            &args.default_image,
            args.clear_default_image,
        )?),
    ))
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

fn resolve_summary_input(args: &ReposSummarySetArgs) -> Result<Option<String>> {
    if args.clear {
        return Ok(None);
    }

    let path = args
        .file
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("--file is required unless --clear is set"))?;
    let contents = read_summary_from_source(path)?;
    validate_summary_contents(&contents)?;
    Ok(Some(contents))
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

async fn resolve_remote_url(
    client: &dyn MetisClientInterface,
    args: &UpdateRepositoryArgs,
) -> Result<String> {
    if let Some(remote_url) = &args.remote_url {
        return parse_required(remote_url, "remote URL");
    }

    let repositories = fetch_repositories(client).await?;
    let repository = repositories
        .into_iter()
        .find(|repository| repository.name == args.name)
        .with_context(|| {
            format!(
                "repository '{}' not found; pass --remote-url to set one",
                args.name
            )
        })?;
    parse_required(&repository.repository.remote_url, "remote URL")
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
        GetRepositoryResponse, ListRepositoriesResponse, Repository, RepositoryRecord,
        SetRepositorySummaryResponse, UpsertRepositoryResponse,
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
                    "default_branch": null,
                    "default_image": "ghcr.io/dourolabs/metis:stable",
                    "content_summary": null
                }));
            then.status(200)
                .json_body_obj(&UpsertRepositoryResponse::new(RepositoryRecord::new(
                    args.name.clone(),
                    Repository::new(
                        "https://example.com/metis.git".to_string(),
                        None,
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
        let args = sample_update_args();

        let error = update_repository(&client, args).await.unwrap_err();
        assert!(
            error.to_string().contains("failed to update repository"),
            "error should include context: {error:?}"
        );

        update_mock.assert();
    }

    #[tokio::test]
    async fn summary_show_prints_markdown_in_pretty_mode() {
        let repo_name = RepoName::from_str("dourolabs/metis").unwrap();
        let response = GetRepositoryResponse::new(RepositoryRecord::new(
            repo_name.clone(),
            Repository::new(
                "https://example.com/metis.git".to_string(),
                Some("main".to_string()),
                None,
                Some("## Summary\nLine".to_string()),
            ),
        ));
        let server = MockServer::start();
        let get_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/repositories/dourolabs/metis");
            then.status(200).json_body_obj(&response);
        });
        let client = mock_client(&server);
        let args = ReposSummaryShowArgs {
            name: repo_name.clone(),
        };
        let context = CommandContext::new(ResolvedOutputFormat::Pretty);
        let mut output = Vec::new();

        summary_show(&client, args, &context, &mut output)
            .await
            .unwrap();

        get_mock.assert();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "## Summary\nLine\n".to_string()
        );
    }

    #[tokio::test]
    async fn summary_show_reports_missing_summary() {
        let repo_name = RepoName::from_str("dourolabs/metis").unwrap();
        let response = GetRepositoryResponse::new(RepositoryRecord::new(
            repo_name.clone(),
            Repository::new(
                "https://example.com/metis.git".to_string(),
                None,
                None,
                None,
            ),
        ));
        let server = MockServer::start();
        let get_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/repositories/dourolabs/metis");
            then.status(200).json_body_obj(&response);
        });
        let client = mock_client(&server);
        let args = ReposSummaryShowArgs {
            name: repo_name.clone(),
        };
        let context = CommandContext::new(ResolvedOutputFormat::Pretty);
        let mut output = Vec::new();

        summary_show(&client, args, &context, &mut output)
            .await
            .unwrap();

        get_mock.assert();
        let rendered = String::from_utf8(output).unwrap();
        assert!(rendered.contains("does not have a content summary"));
    }

    #[tokio::test]
    async fn summary_show_supports_jsonl_output() {
        let repo_name = RepoName::from_str("dourolabs/metis").unwrap();
        let response = GetRepositoryResponse::new(RepositoryRecord::new(
            repo_name.clone(),
            Repository::new(
                "https://example.com/metis.git".to_string(),
                None,
                None,
                Some("## Summary".to_string()),
            ),
        ));
        let server = MockServer::start();
        let get_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/repositories/dourolabs/metis");
            then.status(200).json_body_obj(&response);
        });
        let client = mock_client(&server);
        let args = ReposSummaryShowArgs { name: repo_name };
        let context = CommandContext::new(ResolvedOutputFormat::Jsonl);
        let mut output = Vec::new();

        summary_show(&client, args, &context, &mut output)
            .await
            .unwrap();

        get_mock.assert();
        let rendered = String::from_utf8(output).unwrap();
        assert!(
            rendered.contains("\"content_summary\":\"## Summary\""),
            "{rendered}"
        );
    }

    #[tokio::test]
    async fn summary_set_reads_file_and_prints_status() {
        let repo_name = RepoName::from_str("dourolabs/metis").unwrap();
        let server = MockServer::start();
        let response = SetRepositorySummaryResponse::new(RepositoryRecord::new(
            repo_name.clone(),
            Repository::new(
                "https://example.com/metis.git".to_string(),
                None,
                None,
                Some("## CLI Summary".to_string()),
            ),
        ));
        let set_mock = server.mock(|when, then| {
            when.method(PUT)
                .path("/v1/repositories/dourolabs/metis/content-summary")
                .json_body(json!({ "content_summary": "## CLI Summary" }));
            then.status(200).json_body_obj(&response);
        });
        let client = mock_client(&server);
        let mut file = NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut file, b"## CLI Summary").unwrap();
        let args = ReposSummarySetArgs {
            name: repo_name,
            file: Some(file.path().to_path_buf()),
            clear: false,
        };
        let context = CommandContext::new(ResolvedOutputFormat::Pretty);
        let mut output = Vec::new();

        summary_set(&client, args, &context, &mut output)
            .await
            .unwrap();

        set_mock.assert();
        let rendered = String::from_utf8(output).unwrap();
        assert!(rendered.contains("Updated content summary"));
    }

    #[tokio::test]
    async fn summary_set_requires_input_when_not_clearing() {
        let repo_name = RepoName::from_str("dourolabs/metis").unwrap();
        let client = mock_client(&MockServer::start());
        let args = ReposSummarySetArgs {
            name: repo_name,
            file: None,
            clear: false,
        };
        let context = CommandContext::new(ResolvedOutputFormat::Pretty);
        let mut output = Vec::new();

        let error = summary_set(&client, args, &context, &mut output)
            .await
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("--file is required unless --clear is set"),
            "{error:?}"
        );
    }
}
