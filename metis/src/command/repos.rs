use crate::client::MetisClientInterface;
use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use metis_common::repositories::{
    CreateRepositoryRequest, ServiceRepositoryConfig, ServiceRepositoryInfo,
    UpdateRepositoryRequest,
};
use metis_common::RepoName;
use std::io::{self, Write};

#[derive(Debug, Subcommand)]
pub enum ReposCommand {
    /// List configured repositories.
    List,
    /// Create a new repository configuration.
    Create(UpsertRepositoryArgs),
    /// Update an existing repository configuration.
    Update(UpsertRepositoryArgs),
}

#[derive(Debug, Clone, Args)]
pub struct UpsertRepositoryArgs {
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

    /// GitHub token to use when cloning this repository.
    #[arg(
        long = "github-token",
        value_name = "TOKEN",
        conflicts_with = "clear_github_token"
    )]
    pub github_token: Option<String>,

    /// Remove any configured GitHub token.
    #[arg(long = "clear-github-token")]
    pub clear_github_token: bool,
}

pub async fn run(client: &dyn MetisClientInterface, command: ReposCommand) -> Result<()> {
    match command {
        ReposCommand::List => {
            let repositories = fetch_repositories(client).await?;
            let mut stdout = io::stdout().lock();
            print_repositories(&repositories, &mut stdout)?;
        }
        ReposCommand::Create(args) => {
            let repository = create_repository(client, args).await?;
            let mut stdout = io::stdout().lock();
            print_single_repository("Created repository", &repository, &mut stdout)?;
        }
        ReposCommand::Update(args) => {
            let repository = update_repository(client, args).await?;
            let mut stdout = io::stdout().lock();
            print_single_repository("Updated repository", &repository, &mut stdout)?;
        }
    }

    Ok(())
}

async fn fetch_repositories(
    client: &dyn MetisClientInterface,
) -> Result<Vec<ServiceRepositoryInfo>> {
    let response = client
        .list_repositories()
        .await
        .context("failed to list repositories")?;
    Ok(response.repositories)
}

async fn create_repository(
    client: &dyn MetisClientInterface,
    args: UpsertRepositoryArgs,
) -> Result<ServiceRepositoryInfo> {
    let request = build_create_request(&args)?;
    let response = client
        .create_repository(&request)
        .await
        .context("failed to create repository")?;
    Ok(response.repository)
}

async fn update_repository(
    client: &dyn MetisClientInterface,
    args: UpsertRepositoryArgs,
) -> Result<ServiceRepositoryInfo> {
    let (repo_name, request) = build_update_request(&args)?;
    let response = client
        .update_repository(&repo_name, &request)
        .await
        .context("failed to update repository")?;
    Ok(response.repository)
}

fn build_create_request(args: &UpsertRepositoryArgs) -> Result<CreateRepositoryRequest> {
    Ok(CreateRepositoryRequest::new(
        args.name.clone(),
        build_repository_config(args)?,
    ))
}

fn build_update_request(
    args: &UpsertRepositoryArgs,
) -> Result<(RepoName, UpdateRepositoryRequest)> {
    Ok((
        args.name.clone(),
        UpdateRepositoryRequest::new(build_repository_config(args)?),
    ))
}

fn build_repository_config(args: &UpsertRepositoryArgs) -> Result<ServiceRepositoryConfig> {
    Ok(ServiceRepositoryConfig::new(
        parse_required(&args.remote_url, "remote URL")?,
        parse_optional(
            &args.default_branch,
            args.clear_default_branch,
            "default branch",
            "--clear-default-branch",
        )?,
        parse_optional(
            &args.github_token,
            args.clear_github_token,
            "github token",
            "--clear-github-token",
        )?,
        parse_optional(
            &args.default_image,
            args.clear_default_image,
            "default image",
            "--clear-default-image",
        )?,
    ))
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

fn print_repositories(
    repositories: &[ServiceRepositoryInfo],
    writer: &mut impl Write,
) -> Result<()> {
    if repositories.is_empty() {
        writeln!(writer, "No repositories configured.")?;
        writer.flush()?;
        return Ok(());
    }

    writeln!(writer, "Configured repositories:")?;
    for repository in repositories {
        write_repository_details(repository, "  ", writer)?;
    }
    writer.flush()?;
    Ok(())
}

fn print_single_repository(
    action: &str,
    repository: &ServiceRepositoryInfo,
    writer: &mut impl Write,
) -> Result<()> {
    writeln!(writer, "{action}:")?;
    write_repository_details(repository, "  ", writer)?;
    writer.flush()?;
    Ok(())
}

fn write_repository_details(
    repository: &ServiceRepositoryInfo,
    indent: &str,
    writer: &mut impl Write,
) -> Result<()> {
    writeln!(writer, "{indent}- {}", repository.name)?;
    writeln!(writer, "{indent}  remote_url: {}", repository.remote_url)?;
    writeln!(
        writer,
        "{indent}  default_branch: {}",
        repository.default_branch.as_deref().unwrap_or("<none>")
    )?;
    writeln!(
        writer,
        "{indent}  default_image: {}",
        repository.default_image.as_deref().unwrap_or("<none>")
    )?;
    writeln!(
        writer,
        "{indent}  github_token: {}",
        if repository.github_token_present {
            "set"
        } else {
            "not set"
        }
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MockMetisClient;
    use metis_common::repositories::{ListRepositoriesResponse, UpsertRepositoryResponse};
    use std::str::FromStr;

    fn sample_upsert_args() -> UpsertRepositoryArgs {
        UpsertRepositoryArgs {
            name: RepoName::from_str("dourolabs/metis").unwrap(),
            remote_url: "https://example.com/metis.git".to_string(),
            default_branch: Some("main".to_string()),
            clear_default_branch: false,
            default_image: Some("ghcr.io/dourolabs/metis:latest".to_string()),
            clear_default_image: false,
            github_token: Some("token-123".to_string()),
            clear_github_token: false,
        }
    }

    fn sample_repository_info(name: &RepoName) -> ServiceRepositoryInfo {
        ServiceRepositoryInfo::new(
            name.clone(),
            "https://example.com/metis.git".to_string(),
            Some("main".to_string()),
            Some("ghcr.io/dourolabs/metis:latest".to_string()),
            true,
        )
    }

    #[tokio::test]
    async fn list_repositories_prints_all_fields() {
        let client = MockMetisClient::default();
        let repo_name = RepoName::from_str("dourolabs/metis").unwrap();
        client.push_list_repositories_response(ListRepositoriesResponse::new(vec![
            sample_repository_info(&repo_name),
            ServiceRepositoryInfo::new(
                RepoName::from_str("dourolabs/api").unwrap(),
                "git@github.com:dourolabs/api.git".to_string(),
                None,
                None,
                false,
            ),
        ]));

        let repositories = fetch_repositories(&client).await.unwrap();
        let mut output = Vec::new();
        print_repositories(&repositories, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(output.contains("Configured repositories:"));
        assert!(output.contains("dourolabs/metis"));
        assert!(output.contains("remote_url: https://example.com/metis.git"));
        assert!(output.contains("default_branch: main"));
        assert!(output.contains("default_image: ghcr.io/dourolabs/metis:latest"));
        assert!(output.contains("github_token: set"));
        assert!(output.contains("dourolabs/api"));
        assert!(output.contains("github_token: not set"));
    }

    #[tokio::test]
    async fn list_repositories_reports_client_error() {
        let client = MockMetisClient::default();

        let error = fetch_repositories(&client).await.unwrap_err();
        assert!(
            error.to_string().contains("failed to list repositories"),
            "error should include context: {error:?}"
        );
    }

    #[tokio::test]
    async fn create_repository_sends_request_and_prints_result() {
        let client = MockMetisClient::default();
        let args = sample_upsert_args();
        client.push_create_repository_response(UpsertRepositoryResponse::new(
            sample_repository_info(&args.name),
        ));

        let repository = create_repository(&client, args.clone()).await.unwrap();

        let requests = client.recorded_create_repository_requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.name, args.name);
        assert_eq!(request.repository.remote_url, args.remote_url);
        assert_eq!(
            request.repository.default_branch.as_deref(),
            args.default_branch.as_deref()
        );
        assert_eq!(
            request.repository.default_image.as_deref(),
            args.default_image.as_deref()
        );
        assert_eq!(
            request.repository.github_token.as_deref(),
            args.github_token.as_deref()
        );

        let mut output = Vec::new();
        print_single_repository("Created repository", &repository, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("Created repository:"));
        assert!(output.contains("dourolabs/metis"));
        assert!(output.contains("github_token: set"));
    }

    #[tokio::test]
    async fn create_repository_rejects_empty_remote_url() {
        let client = MockMetisClient::default();
        let mut args = sample_upsert_args();
        args.remote_url = "   ".to_string();

        let error = create_repository(&client, args).await.unwrap_err();
        assert!(
            error.to_string().contains("remote URL must not be empty"),
            "error should mention missing remote URL: {error:?}"
        );
    }

    #[tokio::test]
    async fn update_repository_sends_request_and_allows_clearing_fields() {
        let client = MockMetisClient::default();
        let mut args = sample_upsert_args();
        args.clear_default_branch = true;
        args.default_branch = None;
        args.clear_github_token = true;
        args.github_token = None;
        args.default_image = Some("ghcr.io/dourolabs/metis:stable".to_string());
        client.push_update_repository_response(UpsertRepositoryResponse::new(
            ServiceRepositoryInfo::new(
                args.name.clone(),
                args.remote_url.clone(),
                None,
                args.default_image.clone(),
                false,
            ),
        ));

        let repository = update_repository(&client, args.clone()).await.unwrap();
        let requests = client.recorded_update_repository_requests();
        assert_eq!(requests.len(), 1);
        let (name, request) = &requests[0];
        assert_eq!(name, &args.name);
        assert_eq!(request.repository.remote_url, args.remote_url);
        assert!(request.repository.default_branch.is_none());
        assert_eq!(
            request.repository.default_image.as_deref(),
            args.default_image.as_deref()
        );
        assert!(request.repository.github_token.is_none());

        let mut output = Vec::new();
        print_single_repository("Updated repository", &repository, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("Updated repository:"));
        assert!(output.contains("default_branch: <none>"));
        assert!(output.contains("github_token: not set"));
    }

    #[tokio::test]
    async fn update_repository_reports_client_error() {
        let client = MockMetisClient::default();
        let args = sample_upsert_args();

        let error = update_repository(&client, args).await.unwrap_err();
        assert!(
            error.to_string().contains("failed to update repository"),
            "error should include context: {error:?}"
        );
    }
}
