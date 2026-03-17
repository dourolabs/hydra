use crate::{
    client::HydraClientInterface,
    command::output::{render_repository_records, CommandContext},
    git::clone_repo,
};
use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use hydra_common::repositories::{
    CreateRepositoryRequest, MergeRequestConfig, RepoWorkflowConfig, Repository, RepositoryRecord,
    ReviewRequestConfig, SearchRepositoriesQuery, UpdateRepositoryRequest,
};
use hydra_common::RepoName;
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

    /// Remote git URL reachable by hydra workers.
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

    /// Add a reviewer to the patch workflow. Can be specified multiple times for multiple reviewers.
    #[arg(long = "reviewer", value_name = "ASSIGNEE")]
    pub reviewer: Vec<String>,

    /// Set the merger for the patch workflow.
    #[arg(long = "merger", value_name = "ASSIGNEE")]
    pub merger: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct UpdateRepositoryArgs {
    /// Repository name in the form org/repo.
    #[arg(value_name = "NAME")]
    pub name: RepoName,

    /// Remote git URL reachable by hydra workers.
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

    /// Add a reviewer to the patch workflow. Can be specified multiple times for multiple reviewers.
    #[arg(
        long = "reviewer",
        value_name = "ASSIGNEE",
        conflicts_with = "clear_patch_workflow"
    )]
    pub reviewer: Vec<String>,

    /// Set the merger for the patch workflow.
    #[arg(
        long = "merger",
        value_name = "ASSIGNEE",
        conflicts_with = "clear_patch_workflow"
    )]
    pub merger: Option<String>,

    /// Clear the configured patch workflow.
    #[arg(long = "clear-patch-workflow")]
    pub clear_patch_workflow: bool,
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
    client: &dyn HydraClientInterface,
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
    client: &dyn HydraClientInterface,
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
    client: &dyn HydraClientInterface,
    args: DeleteRepositoryArgs,
) -> Result<RepositoryRecord> {
    let deleted = client
        .delete_repository(&args.name)
        .await
        .context("failed to delete repository")?;
    Ok(deleted)
}

async fn create_repository(
    client: &dyn HydraClientInterface,
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
    client: &dyn HydraClientInterface,
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
    client: &dyn HydraClientInterface,
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
    let mut repo = build_repository_config(
        parse_required(&args.remote_url, "remote URL")?,
        &args.default_branch,
        args.clear_default_branch,
        &args.default_image,
        args.clear_default_image,
    )?;
    repo.patch_workflow = build_patch_workflow(&args.reviewer, &args.merger);
    Ok(CreateRepositoryRequest::new(args.name.clone(), repo))
}

async fn build_update_request(
    client: &dyn HydraClientInterface,
    args: &UpdateRepositoryArgs,
) -> Result<(RepoName, UpdateRepositoryRequest)> {
    let current = fetch_current_repository(client, &args.name).await?;

    let remote_url = match &args.remote_url {
        Some(url) => parse_required(url, "remote URL")?,
        None => current.remote_url,
    };

    let default_branch = if args.clear_default_branch {
        None
    } else {
        match &args.default_branch {
            Some(value) => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    bail!(
                        "default branch must not be empty (use --clear-default-branch to clear it)"
                    );
                }
                Some(trimmed.to_string())
            }
            None => current.default_branch,
        }
    };

    let default_image = if args.clear_default_image {
        None
    } else {
        match &args.default_image {
            Some(value) => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    bail!(
                        "default image must not be empty (use --clear-default-image to clear it)"
                    );
                }
                Some(trimmed.to_string())
            }
            None => current.default_image,
        }
    };

    let patch_workflow = if args.clear_patch_workflow {
        None
    } else {
        let new_workflow = build_patch_workflow(&args.reviewer, &args.merger);
        new_workflow.or(current.patch_workflow)
    };

    let repo = Repository::new(remote_url, default_branch, default_image, patch_workflow);

    Ok((args.name.clone(), UpdateRepositoryRequest::new(repo)))
}

async fn fetch_current_repository(
    client: &dyn HydraClientInterface,
    name: &RepoName,
) -> Result<Repository> {
    let repositories = fetch_repositories(client, false).await?;
    let record = repositories
        .into_iter()
        .find(|r| r.name == *name)
        .with_context(|| format!("repository '{name}' not found; pass --remote-url to set one"))?;
    Ok(record.repository)
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

fn build_patch_workflow(
    reviewers: &[String],
    merger: &Option<String>,
) -> Option<RepoWorkflowConfig> {
    if reviewers.is_empty() && merger.is_none() {
        return None;
    }
    let review_requests = reviewers
        .iter()
        .map(|assignee| ReviewRequestConfig {
            assignee: assignee.clone(),
        })
        .collect();
    let merge_request = merger.as_ref().map(|assignee| MergeRequestConfig {
        assignee: Some(assignee.clone()),
    });
    Some(RepoWorkflowConfig {
        review_requests,
        merge_request,
    })
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
        client::HydraClient,
        command::output::{render_repository_records, ResolvedOutputFormat},
    };
    use httpmock::prelude::*;
    use hydra_common::repositories::{ListRepositoriesResponse, UpsertRepositoryResponse};
    use reqwest::Client as HttpClient;
    use serde_json::json;
    use std::str::FromStr;

    const TEST_HYDRA_TOKEN: &str = "test-hydra-token";

    fn sample_create_args() -> CreateRepositoryArgs {
        CreateRepositoryArgs {
            name: RepoName::from_str("dourolabs/metis").unwrap(),
            remote_url: "https://example.com/hydra.git".to_string(),
            default_branch: Some("main".to_string()),
            clear_default_branch: false,
            default_image: Some("ghcr.io/dourolabs/hydra:latest".to_string()),
            clear_default_image: false,
            reviewer: vec![],
            merger: None,
        }
    }

    fn sample_update_args() -> UpdateRepositoryArgs {
        UpdateRepositoryArgs {
            name: RepoName::from_str("dourolabs/metis").unwrap(),
            remote_url: Some("https://example.com/hydra.git".to_string()),
            default_branch: Some("main".to_string()),
            clear_default_branch: false,
            default_image: Some("ghcr.io/dourolabs/hydra:latest".to_string()),
            clear_default_image: false,
            reviewer: vec![],
            merger: None,
            clear_patch_workflow: false,
        }
    }

    fn sample_repository_info(name: &RepoName) -> RepositoryRecord {
        RepositoryRecord::new(
            name.clone(),
            Repository::new(
                "https://example.com/hydra.git".to_string(),
                Some("main".to_string()),
                Some("ghcr.io/dourolabs/hydra:latest".to_string()),
                None,
            ),
        )
    }

    fn mock_client(server: &MockServer) -> HydraClient {
        HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())
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

        let repositories = fetch_repositories(&client, false).await.unwrap();
        let mut output = Vec::new();
        render_repository_records(ResolvedOutputFormat::Pretty, &repositories, &mut output)
            .unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(output.contains("dourolabs/metis"));
        assert!(output.contains("remote_url: https://example.com/hydra.git"));
        assert!(output.contains("default_branch: main"));
        assert!(output.contains("default_image: ghcr.io/dourolabs/hydra:latest"));
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
                "remote_url": "https://example.com/hydra.git",
                "default_branch": "main",
                "default_image": "ghcr.io/dourolabs/hydra:latest"
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
        args.default_image = Some("ghcr.io/dourolabs/hydra:stable".to_string());
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
                    "remote_url": "https://example.com/hydra.git",
                    "default_branch": null,
                    "default_image": "ghcr.io/dourolabs/hydra:stable"
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

        list_mock.assert();
        update_mock.assert();
    }

    #[tokio::test]
    async fn update_repository_uses_remote_url_from_listing() {
        let mut args = sample_update_args();
        args.remote_url = None;
        args.default_branch = None;
        args.default_image = Some("ghcr.io/dourolabs/hydra:stable".to_string());
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
                    "remote_url": "https://example.com/hydra.git",
                    "default_branch": "main",
                    "default_image": "ghcr.io/dourolabs/hydra:stable"
                }));
            then.status(200)
                .json_body_obj(&UpsertRepositoryResponse::new(RepositoryRecord::new(
                    args.name.clone(),
                    Repository::new(
                        "https://example.com/hydra.git".to_string(),
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
            "https://example.com/hydra.git"
        );
        assert_eq!(
            repository.repository.default_branch,
            Some("main".to_string())
        );
        list_mock.assert();
        update_mock.assert();
    }

    #[tokio::test]
    async fn update_repository_reports_client_error() {
        let args = sample_update_args();
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
                    "remote_url": "https://example.com/hydra.git",
                    "default_branch": "main",
                    "default_image": "ghcr.io/dourolabs/hydra:latest"
                }));
            then.status(404);
        });
        let client = mock_client(&server);

        let error = update_repository(&client, args).await.unwrap_err();
        assert!(
            error.to_string().contains("failed to update repository"),
            "error should include context: {error:?}"
        );

        list_mock.assert();
        update_mock.assert();
    }

    #[tokio::test]
    async fn update_repository_preserves_unmodified_fields() {
        // Only --default-image is provided; default_branch and remote_url should be preserved.
        let mut args = sample_update_args();
        args.remote_url = None;
        args.default_branch = None;
        args.default_image = Some("ghcr.io/dourolabs/hydra:canary".to_string());
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
                    "remote_url": "https://example.com/hydra.git",
                    "default_branch": "main",
                    "default_image": "ghcr.io/dourolabs/hydra:canary"
                }));
            then.status(200)
                .json_body_obj(&UpsertRepositoryResponse::new(RepositoryRecord::new(
                    args.name.clone(),
                    Repository::new(
                        "https://example.com/hydra.git".to_string(),
                        Some("main".to_string()),
                        Some("ghcr.io/dourolabs/hydra:canary".to_string()),
                        None,
                    ),
                )));
        });
        let client = mock_client(&server);

        let repository = update_repository(&client, args).await.unwrap();

        assert_eq!(
            repository.repository.remote_url,
            "https://example.com/hydra.git"
        );
        assert_eq!(
            repository.repository.default_branch,
            Some("main".to_string())
        );
        assert_eq!(
            repository.repository.default_image,
            Some("ghcr.io/dourolabs/hydra:canary".to_string())
        );
        list_mock.assert();
        update_mock.assert();
    }

    #[tokio::test]
    async fn update_repository_clear_default_branch_preserves_default_image() {
        let mut args = sample_update_args();
        args.remote_url = None;
        args.default_branch = None;
        args.clear_default_branch = true;
        args.default_image = None;
        args.clear_default_image = false;
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
                    "remote_url": "https://example.com/hydra.git",
                    "default_branch": null,
                    "default_image": "ghcr.io/dourolabs/hydra:latest"
                }));
            then.status(200)
                .json_body_obj(&UpsertRepositoryResponse::new(RepositoryRecord::new(
                    args.name.clone(),
                    Repository::new(
                        "https://example.com/hydra.git".to_string(),
                        None,
                        Some("ghcr.io/dourolabs/hydra:latest".to_string()),
                        None,
                    ),
                )));
        });
        let client = mock_client(&server);

        let repository = update_repository(&client, args).await.unwrap();

        assert_eq!(repository.repository.default_branch, None);
        assert_eq!(
            repository.repository.default_image,
            Some("ghcr.io/dourolabs/hydra:latest".to_string())
        );
        list_mock.assert();
        update_mock.assert();
    }

    #[tokio::test]
    async fn update_repository_clear_default_image_preserves_default_branch() {
        let mut args = sample_update_args();
        args.remote_url = None;
        args.default_branch = None;
        args.clear_default_branch = false;
        args.default_image = None;
        args.clear_default_image = true;
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
                    "remote_url": "https://example.com/hydra.git",
                    "default_branch": "main",
                    "default_image": null
                }));
            then.status(200)
                .json_body_obj(&UpsertRepositoryResponse::new(RepositoryRecord::new(
                    args.name.clone(),
                    Repository::new(
                        "https://example.com/hydra.git".to_string(),
                        Some("main".to_string()),
                        None,
                        None,
                    ),
                )));
        });
        let client = mock_client(&server);

        let repository = update_repository(&client, args).await.unwrap();

        assert_eq!(
            repository.repository.default_branch,
            Some("main".to_string())
        );
        assert_eq!(repository.repository.default_image, None);
        list_mock.assert();
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

    fn sample_workflow_config() -> RepoWorkflowConfig {
        RepoWorkflowConfig {
            review_requests: vec![ReviewRequestConfig {
                assignee: "alice".to_string(),
            }],
            merge_request: Some(MergeRequestConfig {
                assignee: Some("$patch_creator".to_string()),
            }),
        }
    }

    #[tokio::test]
    async fn create_repository_with_patch_workflow() {
        let mut args = sample_create_args();
        args.reviewer = vec!["alice".to_string()];
        args.merger = Some("$patch_creator".to_string());
        let server = MockServer::start();
        let create_mock = server.mock(|when, then| {
            when.method(POST).path("/v1/repositories").json_body(json!({
                "name": "dourolabs/metis",
                "remote_url": "https://example.com/hydra.git",
                "default_branch": "main",
                "default_image": "ghcr.io/dourolabs/hydra:latest",
                "patch_workflow": {
                    "review_requests": [{"assignee": "alice"}],
                    "merge_request": {"assignee": "$patch_creator"}
                }
            }));
            then.status(200)
                .json_body_obj(&UpsertRepositoryResponse::new(RepositoryRecord::new(
                    args.name.clone(),
                    Repository::new(
                        args.remote_url.clone(),
                        args.default_branch.clone(),
                        args.default_image.clone(),
                        Some(sample_workflow_config()),
                    ),
                )));
        });
        let client = mock_client(&server);

        let repository = create_repository(&client, args).await.unwrap();
        assert!(repository.repository.patch_workflow.is_some());

        create_mock.assert();
    }

    #[tokio::test]
    async fn create_repository_without_patch_workflow() {
        let args = sample_create_args();
        let server = MockServer::start();
        let create_mock = server.mock(|when, then| {
            when.method(POST).path("/v1/repositories").json_body(json!({
                "name": "dourolabs/metis",
                "remote_url": "https://example.com/hydra.git",
                "default_branch": "main",
                "default_image": "ghcr.io/dourolabs/hydra:latest"
            }));
            then.status(200)
                .json_body_obj(&UpsertRepositoryResponse::new(RepositoryRecord::new(
                    args.name.clone(),
                    Repository::new(
                        args.remote_url.clone(),
                        args.default_branch.clone(),
                        args.default_image.clone(),
                        None,
                    ),
                )));
        });
        let client = mock_client(&server);

        let repository = create_repository(&client, args).await.unwrap();
        assert!(repository.repository.patch_workflow.is_none());

        create_mock.assert();
    }

    #[tokio::test]
    async fn update_repository_with_patch_workflow() {
        let mut args = sample_update_args();
        args.reviewer = vec!["alice".to_string()];
        args.merger = Some("$patch_creator".to_string());
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
                    "remote_url": "https://example.com/hydra.git",
                    "default_branch": "main",
                    "default_image": "ghcr.io/dourolabs/hydra:latest",
                    "patch_workflow": {
                        "review_requests": [{"assignee": "alice"}],
                        "merge_request": {"assignee": "$patch_creator"}
                    }
                }));
            then.status(200)
                .json_body_obj(&UpsertRepositoryResponse::new(RepositoryRecord::new(
                    args.name.clone(),
                    Repository::new(
                        args.remote_url.clone().unwrap(),
                        args.default_branch.clone(),
                        args.default_image.clone(),
                        Some(sample_workflow_config()),
                    ),
                )));
        });
        let client = mock_client(&server);

        let repository = update_repository(&client, args).await.unwrap();
        assert!(repository.repository.patch_workflow.is_some());

        list_mock.assert();
        update_mock.assert();
    }

    #[tokio::test]
    async fn update_repository_clear_patch_workflow() {
        let mut args = sample_update_args();
        args.clear_patch_workflow = true;
        let server = MockServer::start();
        let mut existing = sample_repository_info(&args.name);
        existing.repository.patch_workflow = Some(sample_workflow_config());
        let list_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/repositories");
            then.status(200)
                .json_body_obj(&ListRepositoriesResponse::new(vec![existing]));
        });
        let update_mock = server.mock(|when, then| {
            when.method(PUT)
                .path("/v1/repositories/dourolabs/metis")
                .json_body(json!({
                    "remote_url": "https://example.com/hydra.git",
                    "default_branch": "main",
                    "default_image": "ghcr.io/dourolabs/hydra:latest"
                }));
            then.status(200)
                .json_body_obj(&UpsertRepositoryResponse::new(RepositoryRecord::new(
                    args.name.clone(),
                    Repository::new(
                        args.remote_url.clone().unwrap(),
                        args.default_branch.clone(),
                        args.default_image.clone(),
                        None,
                    ),
                )));
        });
        let client = mock_client(&server);

        let repository = update_repository(&client, args).await.unwrap();
        assert!(repository.repository.patch_workflow.is_none());

        list_mock.assert();
        update_mock.assert();
    }

    #[tokio::test]
    async fn update_repository_preserves_patch_workflow_when_unmodified() {
        let mut args = sample_update_args();
        args.default_image = Some("ghcr.io/dourolabs/hydra:canary".to_string());
        let server = MockServer::start();
        let mut existing = sample_repository_info(&args.name);
        existing.repository.patch_workflow = Some(sample_workflow_config());
        let list_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/repositories");
            then.status(200)
                .json_body_obj(&ListRepositoriesResponse::new(vec![existing]));
        });
        let update_mock = server.mock(|when, then| {
            when.method(PUT)
                .path("/v1/repositories/dourolabs/metis")
                .json_body(json!({
                    "remote_url": "https://example.com/hydra.git",
                    "default_branch": "main",
                    "default_image": "ghcr.io/dourolabs/hydra:canary",
                    "patch_workflow": {
                        "review_requests": [{"assignee": "alice"}],
                        "merge_request": {"assignee": "$patch_creator"}
                    }
                }));
            then.status(200)
                .json_body_obj(&UpsertRepositoryResponse::new(RepositoryRecord::new(
                    args.name.clone(),
                    Repository::new(
                        args.remote_url.clone().unwrap(),
                        args.default_branch.clone(),
                        args.default_image.clone(),
                        Some(sample_workflow_config()),
                    ),
                )));
        });
        let client = mock_client(&server);

        let repository = update_repository(&client, args).await.unwrap();
        assert!(repository.repository.patch_workflow.is_some());

        list_mock.assert();
        update_mock.assert();
    }

    #[test]
    fn list_repositories_shows_patch_workflow_in_pretty_output() {
        let repo_name = RepoName::from_str("dourolabs/metis").unwrap();
        let mut repo_info = sample_repository_info(&repo_name);
        repo_info.repository.patch_workflow = Some(sample_workflow_config());

        let mut output = Vec::new();
        render_repository_records(ResolvedOutputFormat::Pretty, &[repo_info], &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(output.contains("reviewers: alice"));
        assert!(output.contains("merger: $patch_creator"));
    }

    #[test]
    fn list_repositories_omits_patch_workflow_when_none() {
        let repo_name = RepoName::from_str("dourolabs/metis").unwrap();
        let repo_info = sample_repository_info(&repo_name);

        let mut output = Vec::new();
        render_repository_records(ResolvedOutputFormat::Pretty, &[repo_info], &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(!output.contains("reviewers:"));
        assert!(!output.contains("merger:"));
    }
}
