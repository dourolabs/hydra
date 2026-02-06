use crate::{
    client::MetisClientInterface,
    command::output::{render_repository_records, CommandContext},
    git::clone_repo,
};
use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use metis_common::repositories::{
    CreateRepositoryRequest, Repository, RepositoryRecord, SearchRepositoriesQuery,
    UpdateRepositoryRequest,
};
use metis_common::RepoName;
use std::io;
use std::path::PathBuf;

#[derive(Debug, Subcommand)]
pub enum ReposCommand {
    /// List configured repositories.
    List(ListRepositoryArgs),
    /// Create a new repository configuration.
    Create(CreateRepositoryArgs),
    /// Update an existing repository configuration.
    Update(UpdateRepositoryArgs),
    /// Delete (soft-delete) a repository configuration.
    Delete(DeleteRepositoryArgs),
    /// Clone a repository to a local directory.
    Clone(CloneRepositoryArgs),
}

#[derive(Debug, Clone, Args)]
pub struct ListRepositoryArgs {
    /// Include deleted repositories in the list.
    #[arg(long = "include-deleted")]
    pub include_deleted: bool,
}

#[derive(Debug, Clone, Args)]
pub struct DeleteRepositoryArgs {
    /// Repository name in the form org/repo.
    #[arg(value_name = "NAME")]
    pub name: RepoName,
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
pub struct CloneRepositoryArgs {
    /// Repository name in the form org/repo.
    #[arg(value_name = "NAME")]
    pub name: RepoName,

    /// Target directory to clone into. Defaults to the repository name.
    #[arg(value_name = "DIRECTORY")]
    pub directory: Option<PathBuf>,

    /// Revision to checkout (branch, tag, or commit SHA). Defaults to the repository's default branch or HEAD.
    #[arg(long = "rev", value_name = "REVISION")]
    pub revision: Option<String>,
}

pub async fn run(
    client: &dyn MetisClientInterface,
    command: ReposCommand,
    context: &CommandContext,
) -> Result<()> {
    let mut stdout = io::stdout().lock();
    match command {
        ReposCommand::List(args) => {
            let repositories = fetch_repositories(client, args.include_deleted).await?;
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
        ReposCommand::Delete(args) => {
            let repository = delete_repository(client, args).await?;
            render_repository_records(context.output_format, &[repository], &mut stdout)?;
        }
        ReposCommand::Clone(args) => {
            clone_repository(client, args).await?;
        }
    }

    Ok(())
}

async fn fetch_repositories(
    client: &dyn MetisClientInterface,
    include_deleted: bool,
) -> Result<Vec<RepositoryRecord>> {
    let query = SearchRepositoriesQuery::new(if include_deleted { Some(true) } else { None });
    let response = client
        .list_repositories(&query)
        .await
        .context("failed to list repositories")?;
    Ok(response.repositories)
}

async fn delete_repository(
    client: &dyn MetisClientInterface,
    args: DeleteRepositoryArgs,
) -> Result<RepositoryRecord> {
    let deleted = client
        .delete_repository(&args.name)
        .await
        .context("failed to delete repository")?;
    Ok(deleted)
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

async fn clone_repository(
    client: &dyn MetisClientInterface,
    args: CloneRepositoryArgs,
) -> Result<()> {
    let repositories = fetch_repositories(client, false).await?;
    let repository = repositories
        .into_iter()
        .find(|r| r.name == args.name)
        .with_context(|| format!("repository '{}' not found", args.name))?;

    let remote_url = &repository.repository.remote_url;
    let revision = args
        .revision
        .or(repository.repository.default_branch)
        .unwrap_or_else(|| "HEAD".to_string());

    let destination = args
        .directory
        .unwrap_or_else(|| PathBuf::from(args.name.to_string()));

    let github_token = client.get_github_token().await.ok();

    clone_repo(remote_url, &revision, &destination, github_token.as_deref()).with_context(
        || {
            format!(
                "failed to clone repository '{}' to '{}'",
                args.name,
                destination.display()
            )
        },
    )?;

    eprintln!("Cloned {} to {}", args.name, destination.display());
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
    ))
}

async fn resolve_remote_url(
    client: &dyn MetisClientInterface,
    args: &UpdateRepositoryArgs,
) -> Result<String> {
    if let Some(remote_url) = &args.remote_url {
        return parse_required(remote_url, "remote URL");
    }

    let repositories = fetch_repositories(client, false).await?;
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
        command::output::{render_repository_records, ResolvedOutputFormat},
    };
    use httpmock::prelude::*;
    use metis_common::repositories::{ListRepositoriesResponse, UpsertRepositoryResponse};
    use reqwest::Client as HttpClient;
    use serde_json::json;
    use std::str::FromStr;

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
                Repository::new("git@github.com:dourolabs/api.git".to_string(), None, None),
            ),
        ]);
        let server = MockServer::start();
        let list_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/repositories");
            then.status(200).json_body_obj(&repositories);
        });
        let client = mock_client(&server);

        let repositories = fetch_repositories(&client, false).await.unwrap();
        let mut output = Vec::new();
        render_repository_records(ResolvedOutputFormat::Pretty, &repositories, &mut output)
            .unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(output.contains("dourolabs/metis"));
        assert!(output.contains("remote_url: https://example.com/metis.git"));
        assert!(output.contains("default_branch: main"));
        assert!(output.contains("default_image: ghcr.io/dourolabs/metis:latest"));
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

        let error = fetch_repositories(&client, false).await.unwrap_err();
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
                "default_image": "ghcr.io/dourolabs/metis:latest"
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
                    "default_image": "ghcr.io/dourolabs/metis:stable"
                }));
            then.status(200)
                .json_body_obj(&UpsertRepositoryResponse::new(RepositoryRecord::new(
                    args.name.clone(),
                    Repository::new(
                        args.remote_url.clone().unwrap(),
                        None,
                        args.default_image.clone(),
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
                    "default_image": "ghcr.io/dourolabs/metis:stable"
                }));
            then.status(200)
                .json_body_obj(&UpsertRepositoryResponse::new(RepositoryRecord::new(
                    args.name.clone(),
                    Repository::new(
                        "https://example.com/metis.git".to_string(),
                        None,
                        args.default_image.clone(),
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
                    "default_image": "ghcr.io/dourolabs/metis:latest"
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

    fn sample_clone_args() -> CloneRepositoryArgs {
        CloneRepositoryArgs {
            name: RepoName::from_str("dourolabs/metis").unwrap(),
            directory: None,
            revision: None,
        }
    }

    #[tokio::test]
    async fn clone_repository_reports_not_found() {
        let server = MockServer::start();
        let list_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/repositories");
            then.status(200)
                .json_body_obj(&ListRepositoriesResponse::new(vec![]));
        });
        let client = mock_client(&server);

        let error = clone_repository(&client, sample_clone_args())
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("not found"),
            "error should indicate repo not found: {error:?}"
        );

        list_mock.assert();
    }

    #[test]
    fn clone_args_defaults_directory_to_repo_name() {
        let args = sample_clone_args();
        let destination = args
            .directory
            .clone()
            .unwrap_or_else(|| PathBuf::from(args.name.to_string()));
        assert_eq!(destination, PathBuf::from("dourolabs/metis"));
    }

    #[test]
    fn clone_args_uses_provided_directory() {
        let mut args = sample_clone_args();
        args.directory = Some(PathBuf::from("/tmp/my-clone"));
        let destination = args
            .directory
            .clone()
            .unwrap_or_else(|| PathBuf::from(args.name.to_string()));
        assert_eq!(destination, PathBuf::from("/tmp/my-clone"));
    }
}
