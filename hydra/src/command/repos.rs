use crate::{
    client::HydraClient,
    command::output::{render, CommandContext, RepositoryRecords},
    git::clone_repo,
    output_writer::write_stdout,
};
use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use hydra_common::repositories::{
    CreateRepositoryRequest, MergePolicy, Repository, RepositoryRecord, SearchRepositoriesQuery,
    UpdateRepositoryRequest,
};
use hydra_common::RepoName;
use std::path::{Path, PathBuf};

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

    /// Read a YAML merge policy from PATH and apply it to the repository.
    ///
    /// The file is deserialised into a `MergePolicy`; strings starting with
    /// `@` are parsed as dynamic principal references (e.g. `@patch.creator`).
    #[arg(
        long = "merge-policy-file",
        value_name = "PATH",
        conflicts_with = "clear_merge_policy"
    )]
    pub merge_policy_file: Option<PathBuf>,

    /// Clear the configured merge policy.
    #[arg(long = "clear-merge-policy")]
    pub clear_merge_policy: bool,
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
    client: &HydraClient,
    command: ReposCommand,
    context: &CommandContext,
) -> Result<()> {
    let mut buffer = Vec::new();
    match command {
        ReposCommand::List(args) => {
            let repositories = fetch_repositories(client, args.include_deleted).await?;
            render(
                RepositoryRecords(&repositories),
                context.output_format,
                &mut buffer,
            )?;
        }
        ReposCommand::Create(args) => {
            let repository = create_repository(client, args).await?;
            render(
                RepositoryRecords(&[repository]),
                context.output_format,
                &mut buffer,
            )?;
        }
        ReposCommand::Update(args) => {
            let repository = update_repository(client, args).await?;
            render(
                RepositoryRecords(&[repository]),
                context.output_format,
                &mut buffer,
            )?;
        }
        ReposCommand::Delete(args) => {
            let repository = delete_repository(client, args).await?;
            render(
                RepositoryRecords(&[repository]),
                context.output_format,
                &mut buffer,
            )?;
        }
        ReposCommand::Clone(args) => {
            clone_repository(client, args).await?;
        }
    }
    write_stdout(&buffer)?;

    Ok(())
}

async fn fetch_repositories(
    client: &HydraClient,
    include_deleted: bool,
) -> Result<Vec<RepositoryRecord>> {
    let query = SearchRepositoriesQuery::new(if include_deleted { Some(true) } else { None }, None);
    let response = client
        .list_repositories(&query)
        .await
        .context("failed to list repositories")?;
    Ok(response.repositories)
}

async fn delete_repository(
    client: &HydraClient,
    args: DeleteRepositoryArgs,
) -> Result<RepositoryRecord> {
    let deleted = client
        .delete_repository(&args.name)
        .await
        .context("failed to delete repository")?;
    Ok(deleted)
}

async fn create_repository(
    client: &HydraClient,
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
    client: &HydraClient,
    args: UpdateRepositoryArgs,
) -> Result<RepositoryRecord> {
    let (repo_name, request) = build_update_request(client, &args).await?;
    let response = client
        .update_repository(&repo_name, &request)
        .await
        .context("failed to update repository")?;
    Ok(response.repository)
}

async fn clone_repository(client: &HydraClient, args: CloneRepositoryArgs) -> Result<()> {
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
    let repo = build_repository_config(
        resolve_remote_url(&args.remote_url)?,
        &args.default_branch,
        args.clear_default_branch,
        &args.default_image,
        args.clear_default_image,
    )?;
    Ok(CreateRepositoryRequest::new(args.name.clone(), repo))
}

async fn build_update_request(
    client: &HydraClient,
    args: &UpdateRepositoryArgs,
) -> Result<(RepoName, UpdateRepositoryRequest)> {
    let current = fetch_current_repository(client, &args.name).await?;

    let remote_url = match &args.remote_url {
        Some(url) => resolve_remote_url(url)?,
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

    let merge_policy = if args.clear_merge_policy {
        None
    } else if let Some(path) = &args.merge_policy_file {
        Some(load_merge_policy_file(path)?)
    } else {
        current.merge_policy
    };

    let mut repo = Repository::new(remote_url, default_branch, default_image);
    repo.merge_policy = merge_policy;

    Ok((args.name.clone(), UpdateRepositoryRequest::new(repo)))
}

fn load_merge_policy_file(path: &Path) -> Result<MergePolicy> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read merge policy file '{}'", path.display()))?;
    serde_yaml_ng::from_str::<MergePolicy>(&contents)
        .with_context(|| format!("failed to parse merge policy YAML at '{}'", path.display()))
}

async fn fetch_current_repository(client: &HydraClient, name: &RepoName) -> Result<Repository> {
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
    ))
}

/// Detects whether `remote_url` looks like a filesystem path (starts with `/` or `.`)
/// and converts it to a `file://` URL. Validates that the path exists and is a git
/// repository. Non-path values (e.g. `https://`, `git@`) are returned unchanged.
fn resolve_remote_url(remote_url: &str) -> Result<String> {
    let trimmed = remote_url.trim();
    if trimmed.is_empty() {
        bail!("remote URL must not be empty");
    }

    if !trimmed.starts_with('/') && !trimmed.starts_with('.') {
        return Ok(trimmed.to_string());
    }

    let path = Path::new(trimmed);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("failed to determine current directory")?
            .join(path)
    };

    let canonical = absolute
        .canonicalize()
        .with_context(|| format!("path '{trimmed}' does not exist"))?;

    if !is_git_repository(&canonical) {
        bail!(
            "path '{}' exists but is not a git repository (no .git directory found and not a bare repo)",
            canonical.display()
        );
    }

    Ok(format!("file://{}", canonical.display()))
}

/// Returns true if `path` looks like a git repository — either it contains a `.git`
/// directory/file, or it is a bare repo (has a `HEAD` file and `objects` directory).
fn is_git_repository(path: &Path) -> bool {
    if path.join(".git").exists() {
        return true;
    }
    // Bare repo check: HEAD file + objects dir
    path.join("HEAD").exists() && path.join("objects").exists()
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
        command::output::{render, RepositoryRecords, ResolvedOutputFormat},
    };
    use httpmock::prelude::*;
    use hydra_common::repositories::{ListRepositoriesResponse, UpsertRepositoryResponse};
    use reqwest::Client as HttpClient;
    use serde_json::json;
    use std::str::FromStr;

    const TEST_HYDRA_TOKEN: &str = "test-hydra-token";

    fn sample_create_args() -> CreateRepositoryArgs {
        CreateRepositoryArgs {
            name: RepoName::from_str("dourolabs/hydra").unwrap(),
            remote_url: "https://example.com/hydra.git".to_string(),
            default_branch: Some("main".to_string()),
            clear_default_branch: false,
            default_image: Some("ghcr.io/dourolabs/hydra:latest".to_string()),
            clear_default_image: false,
        }
    }

    fn sample_update_args() -> UpdateRepositoryArgs {
        UpdateRepositoryArgs {
            name: RepoName::from_str("dourolabs/hydra").unwrap(),
            remote_url: Some("https://example.com/hydra.git".to_string()),
            default_branch: Some("main".to_string()),
            clear_default_branch: false,
            default_image: Some("ghcr.io/dourolabs/hydra:latest".to_string()),
            clear_default_image: false,
            merge_policy_file: None,
            clear_merge_policy: false,
        }
    }

    fn sample_repository_info(name: &RepoName) -> RepositoryRecord {
        RepositoryRecord::new(
            name.clone(),
            Repository::new(
                "https://example.com/hydra.git".to_string(),
                Some("main".to_string()),
                Some("ghcr.io/dourolabs/hydra:latest".to_string()),
            ),
        )
    }

    fn mock_client(server: &MockServer) -> HydraClient {
        HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())
            .expect("mock client creation should not fail")
    }

    #[tokio::test]
    async fn list_repositories_prints_all_fields() {
        let repo_name = RepoName::from_str("dourolabs/hydra").unwrap();
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
        render(
            RepositoryRecords(&repositories),
            ResolvedOutputFormat::Pretty,
            &mut output,
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(output.contains("dourolabs/hydra"));
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
                "name": "dourolabs/hydra",
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
        render(
            RepositoryRecords(&[repository]),
            ResolvedOutputFormat::Pretty,
            &mut output,
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("dourolabs/hydra"));

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
                .path("/v1/repositories/dourolabs/hydra")
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
                    ),
                )));
        });
        let client = mock_client(&server);

        let repository = update_repository(&client, args.clone()).await.unwrap();

        let mut output = Vec::new();
        render(
            RepositoryRecords(&[repository]),
            ResolvedOutputFormat::Pretty,
            &mut output,
        )
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
                .path("/v1/repositories/dourolabs/hydra")
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
                .path("/v1/repositories/dourolabs/hydra")
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
                .path("/v1/repositories/dourolabs/hydra")
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
                .path("/v1/repositories/dourolabs/hydra")
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
                .path("/v1/repositories/dourolabs/hydra")
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
            name: RepoName::from_str("dourolabs/hydra").unwrap(),
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
        assert_eq!(destination, PathBuf::from("dourolabs/hydra"));
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

    #[test]
    fn resolve_remote_url_rejects_empty() {
        let err = resolve_remote_url("   ").unwrap_err();
        assert!(
            err.to_string().contains("remote URL must not be empty"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn resolve_remote_url_rejects_nonexistent_path() {
        let err = resolve_remote_url("/nonexistent/path/to/repo").unwrap_err();
        assert!(
            err.to_string().contains("does not exist"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn resolve_remote_url_rejects_non_git_directory() {
        let dir = tempfile::tempdir().unwrap();
        let err = resolve_remote_url(dir.path().to_str().unwrap()).unwrap_err();
        assert!(
            err.to_string().contains("not a git repository"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn resolve_remote_url_converts_absolute_git_path() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        let url = resolve_remote_url(dir.path().to_str().unwrap()).unwrap();
        assert!(
            url.starts_with("file://"),
            "should start with file://: {url}"
        );
        assert!(
            url.contains(dir.path().to_str().unwrap()),
            "should contain original path: {url}"
        );
    }

    #[test]
    fn resolve_remote_url_converts_bare_repo_path() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::create_dir(dir.path().join("objects")).unwrap();
        let url = resolve_remote_url(dir.path().to_str().unwrap()).unwrap();
        assert!(
            url.starts_with("file://"),
            "should recognize bare repo: {url}"
        );
    }

    #[test]
    fn is_git_repository_detects_dotgit_directory() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_git_repository(dir.path()));
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        assert!(is_git_repository(dir.path()));
    }

    #[test]
    fn is_git_repository_detects_bare_repo() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_git_repository(dir.path()));
        std::fs::write(dir.path().join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::create_dir(dir.path().join("objects")).unwrap();
        assert!(is_git_repository(dir.path()));
    }

    // ---- merge_policy CLI surface --------------------------------------

    /// Demonstrates the canonical `MergePolicy` shape: two `ReviewerGroup`s
    /// (one `code-review` group with `exclude_author: true` and a
    /// `human-signoff` group without it), each requiring quorum of `count`
    /// approvers from its `any_of` list, plus a `mergers.any_of` that
    /// includes the patch creator via the `@patch.creator` placeholder.
    const SAMPLE_MERGE_POLICY_YAML: &str = r#"
reviewers:
  - label: code-review
    any_of:
      - reviewer
      - carol
    count: 1
    exclude_author: true
  - label: human-signoff
    any_of:
      - alice
      - bob
    count: 1

mergers:
  any_of:
    - "@patch.creator"
    - alice
"#;

    #[test]
    fn load_merge_policy_file_parses_design_example() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("policy.yaml");
        std::fs::write(&path, SAMPLE_MERGE_POLICY_YAML).unwrap();

        let policy = load_merge_policy_file(&path).unwrap();
        assert_eq!(policy.reviewers.len(), 2);
        assert_eq!(policy.reviewers[0].label.as_deref(), Some("code-review"));
        assert_eq!(policy.reviewers[1].label.as_deref(), Some("human-signoff"));
        assert!(policy.mergers.is_some());
    }

    #[test]
    fn load_merge_policy_file_reports_missing_file() {
        let err = load_merge_policy_file(Path::new("/nonexistent/policy.yaml")).unwrap_err();
        assert!(
            err.to_string().contains("failed to read merge policy file"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_merge_policy_file_reports_invalid_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("policy.yaml");
        std::fs::write(&path, "reviewers: [not a sequence element").unwrap();

        let err = load_merge_policy_file(&path).unwrap_err();
        assert!(
            err.to_string()
                .contains("failed to parse merge policy YAML"),
            "unexpected error: {err}"
        );
    }

    /// Regression test for the e2e bootstrap fixture (tests/e2e/run.sh applies
    /// this policy to the test-fixture repo). If a future schema change breaks
    /// this YAML, we want a unit-test failure rather than a runtime failure in
    /// `run.sh`.
    #[test]
    fn load_merge_policy_file_parses_e2e_fixture() {
        use hydra_common::api::v1::agents::AgentName;
        use hydra_common::repositories::AssigneeRef;
        use hydra_common::Principal;

        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root")
            .join("tests/e2e/config/merge-policy.yaml");
        let policy = load_merge_policy_file(&path).expect("e2e fixture must parse");
        assert_eq!(policy.reviewers.len(), 1, "exactly one reviewer group");
        let group = &policy.reviewers[0];
        assert_eq!(
            group.any_of,
            vec![AssigneeRef::Static(Principal::Agent {
                name: AgentName::try_new("reviewer").unwrap(),
            })]
        );
        assert_eq!(group.count, 1);
        assert!(group.exclude_author);
        assert!(
            policy.mergers.is_none(),
            "mergers omitted == anyone may merge"
        );
    }

    #[tokio::test]
    async fn update_repository_with_merge_policy_file() {
        let dir = tempfile::tempdir().unwrap();
        let policy_path = dir.path().join("policy.yaml");
        std::fs::write(&policy_path, SAMPLE_MERGE_POLICY_YAML).unwrap();

        let mut args = sample_update_args();
        args.merge_policy_file = Some(policy_path);
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
                .path("/v1/repositories/dourolabs/hydra")
                .json_body(json!({
                    "remote_url": "https://example.com/hydra.git",
                    "default_branch": "main",
                    "default_image": "ghcr.io/dourolabs/hydra:latest",
                    "merge_policy": {
                        "reviewers": [
                            {
                                "label": "code-review",
                                "any_of": ["users/reviewer", "users/carol"],
                            },
                            {
                                "label": "human-signoff",
                                "any_of": ["users/alice", "users/bob"],
                            }
                        ],
                        "mergers": {
                            "any_of": ["@patch.creator", "users/alice"]
                        }
                    }
                }));
            let response_repo = {
                let mut r = Repository::new(
                    args.remote_url.clone().unwrap(),
                    args.default_branch.clone(),
                    args.default_image.clone(),
                );
                r.merge_policy = Some(serde_yaml_ng::from_str(SAMPLE_MERGE_POLICY_YAML).unwrap());
                r
            };
            then.status(200)
                .json_body_obj(&UpsertRepositoryResponse::new(RepositoryRecord::new(
                    args.name.clone(),
                    response_repo,
                )));
        });
        let client = mock_client(&server);

        let repository = update_repository(&client, args).await.unwrap();
        assert!(repository.repository.merge_policy.is_some());

        list_mock.assert();
        update_mock.assert();
    }

    #[tokio::test]
    async fn update_repository_clear_merge_policy_drops_only_merge_policy() {
        let mut args = sample_update_args();
        args.clear_merge_policy = true;
        let server = MockServer::start();
        let mut existing = sample_repository_info(&args.name);
        existing.repository.merge_policy =
            Some(serde_yaml_ng::from_str(SAMPLE_MERGE_POLICY_YAML).unwrap());
        let list_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/repositories");
            then.status(200)
                .json_body_obj(&ListRepositoriesResponse::new(vec![existing]));
        });
        let update_mock = server.mock(|when, then| {
            when.method(PUT)
                .path("/v1/repositories/dourolabs/hydra")
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
                    ),
                )));
        });
        let client = mock_client(&server);

        let repository = update_repository(&client, args).await.unwrap();
        assert!(repository.repository.merge_policy.is_none());

        list_mock.assert();
        update_mock.assert();
    }

    #[tokio::test]
    async fn update_repository_preserves_merge_policy_when_unmodified() {
        let mut args = sample_update_args();
        args.default_image = Some("ghcr.io/dourolabs/hydra:canary".to_string());
        let policy: MergePolicy = serde_yaml_ng::from_str(SAMPLE_MERGE_POLICY_YAML).unwrap();
        let server = MockServer::start();
        let mut existing = sample_repository_info(&args.name);
        existing.repository.merge_policy = Some(policy.clone());
        let list_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/repositories");
            then.status(200)
                .json_body_obj(&ListRepositoriesResponse::new(vec![existing]));
        });
        let update_mock = server.mock(|when, then| {
            when.method(PUT)
                .path("/v1/repositories/dourolabs/hydra")
                .json_body(json!({
                    "remote_url": "https://example.com/hydra.git",
                    "default_branch": "main",
                    "default_image": "ghcr.io/dourolabs/hydra:canary",
                    "merge_policy": serde_json::to_value(&policy).unwrap(),
                }));
            let mut response_repo = Repository::new(
                args.remote_url.clone().unwrap(),
                args.default_branch.clone(),
                Some("ghcr.io/dourolabs/hydra:canary".to_string()),
            );
            response_repo.merge_policy = Some(policy.clone());
            then.status(200)
                .json_body_obj(&UpsertRepositoryResponse::new(RepositoryRecord::new(
                    args.name.clone(),
                    response_repo,
                )));
        });
        let client = mock_client(&server);

        let repository = update_repository(&client, args).await.unwrap();
        assert!(repository.repository.merge_policy.is_some());

        list_mock.assert();
        update_mock.assert();
    }

    #[test]
    fn list_repositories_shows_merge_policy_in_pretty_output() {
        let repo_name = RepoName::from_str("dourolabs/hydra").unwrap();
        let mut repo_info = sample_repository_info(&repo_name);
        repo_info.repository.merge_policy =
            Some(serde_yaml_ng::from_str(SAMPLE_MERGE_POLICY_YAML).unwrap());

        let mut output = Vec::new();
        render(
            RepositoryRecords(&[repo_info]),
            ResolvedOutputFormat::Pretty,
            &mut output,
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(
            output.contains("merge_policy:"),
            "should print merge_policy header, got:\n{output}"
        );
        assert!(
            output.contains("code-review"),
            "should print policy contents, got:\n{output}"
        );
        assert!(
            output.contains("@patch.creator"),
            "should retain dynamic-ref shorthand, got:\n{output}"
        );
    }

    #[test]
    fn list_repositories_omits_merge_policy_when_none() {
        let repo_name = RepoName::from_str("dourolabs/hydra").unwrap();
        let repo_info = sample_repository_info(&repo_name);

        let mut output = Vec::new();
        render(
            RepositoryRecords(&[repo_info]),
            ResolvedOutputFormat::Pretty,
            &mut output,
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(!output.contains("merge_policy"));
    }
}
