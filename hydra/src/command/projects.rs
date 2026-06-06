use crate::{
    client::HydraClientInterface,
    command::{
        output::{render, CommandContext, ProjectRecords, ProjectStatuses, ResolvedOutputFormat},
        project_body_file::load_body_file,
        utils::resolve_username,
    },
    output_writer::write_stdout,
};
use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use hydra_common::api::v1::projects::{
    Project, ProjectIdOrDefault, ProjectKey, ProjectRecord, StatusKey, UpsertProjectRequest,
    UpsertProjectResponse,
};
use hydra_common::ProjectId;
use std::path::PathBuf;
use std::str::FromStr;

#[derive(Debug, Subcommand)]
pub enum ProjectsCommand {
    /// List configured projects.
    List,
    /// Create a new project.
    Create(CreateProjectArgs),
    /// Get a project by its id.
    Get(GetProjectArgs),
    /// Replace an existing project (full update).
    Update(UpdateProjectArgs),
    /// Soft-delete a project.
    Delete(DeleteProjectArgs),
    /// List the status definitions for a project. Pass `default` for the
    /// synthesized default project's statuses.
    Statuses(StatusesProjectArgs),
    /// Operate on a single status within a project.
    Status {
        #[command(subcommand)]
        command: StatusCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum StatusCommand {
    /// Update fields on a single status within a project.
    Update(UpdateStatusArgs),
}

#[derive(Debug, Clone, Args)]
pub struct CreateProjectArgs {
    /// Project key (unique slug; lowercase letters, digits, and `-`).
    #[arg(long, value_name = "KEY")]
    pub key: ProjectKey,

    /// Human-readable project name.
    #[arg(long, value_name = "NAME")]
    pub name: String,

    /// Path to a JSON or YAML file containing the project body: a
    /// `statuses` list (the project-specific set of issue statuses) and a
    /// `default_status_key` selecting the status applied to issues that
    /// don't declare one explicitly. `default_status_key` must reference a
    /// key in the `statuses` list.
    #[arg(long = "body-file", value_name = "PATH")]
    pub body_file: PathBuf,
}

#[derive(Debug, Clone, Args)]
pub struct GetProjectArgs {
    /// Project id (e.g. `j-abc123`).
    #[arg(value_name = "PROJECT_ID")]
    pub project_id: ProjectId,
}

#[derive(Debug, Clone, Args)]
pub struct UpdateProjectArgs {
    /// Project id to update.
    #[arg(value_name = "PROJECT_ID")]
    pub project_id: ProjectId,

    /// New project key. Defaults to the existing value.
    #[arg(long, value_name = "KEY")]
    pub key: Option<ProjectKey>,

    /// New human-readable name. Defaults to the existing value.
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,

    /// Path to a JSON or YAML file containing the new body (`statuses` list
    /// and `default_status_key`). Defaults to the existing body.
    #[arg(long = "body-file", value_name = "PATH")]
    pub body_file: Option<PathBuf>,

    /// Doc-store path for the project-layer prompt slice. Omit to leave
    /// the existing value unchanged; pass `--prompt-path ""` to clear it.
    /// Non-empty values should be absolute doc-store paths starting with
    /// `/` (the server is authoritative).
    #[arg(long = "prompt-path", value_name = "PATH")]
    pub prompt_path: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct UpdateStatusArgs {
    /// Project id whose status is being updated.
    #[arg(value_name = "PROJECT_ID")]
    pub project_id: ProjectId,

    /// Status key (within the project) to update.
    #[arg(value_name = "STATUS_KEY")]
    pub status_key: StatusKey,

    /// Doc-store path for this status's prompt slice. Omit to leave the
    /// existing value unchanged; pass `--prompt-path ""` to clear it.
    /// Non-empty values should be absolute doc-store paths starting with
    /// `/` (the server is authoritative).
    #[arg(long = "prompt-path", value_name = "PATH")]
    pub prompt_path: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct DeleteProjectArgs {
    /// Project id to delete.
    #[arg(value_name = "PROJECT_ID")]
    pub project_id: ProjectId,
}

#[derive(Debug, Clone, Args)]
pub struct StatusesProjectArgs {
    /// Project id or the literal `default` for the synthesized default
    /// project.
    #[arg(value_name = "PROJECT_ID_OR_DEFAULT")]
    pub project: ProjectIdOrDefault,
}

pub async fn run(
    client: &dyn HydraClientInterface,
    command: ProjectsCommand,
    context: &CommandContext,
) -> Result<()> {
    let mut buffer = Vec::new();
    match command {
        ProjectsCommand::List => {
            let projects = client
                .list_projects()
                .await
                .context("failed to list projects")?
                .projects;
            render(
                ProjectRecords(&projects),
                context.output_format,
                &mut buffer,
            )?;
        }
        ProjectsCommand::Create(args) => {
            let record = create_project(client, args).await?;
            render(
                ProjectRecords(&[record]),
                context.output_format,
                &mut buffer,
            )?;
        }
        ProjectsCommand::Get(args) => {
            let record = client
                .get_project(&args.project_id)
                .await
                .with_context(|| format!("failed to fetch project '{}'", args.project_id))?;
            render(
                ProjectRecords(&[record]),
                context.output_format,
                &mut buffer,
            )?;
        }
        ProjectsCommand::Update(args) => {
            let record = update_project(client, args).await?;
            render(
                ProjectRecords(&[record]),
                context.output_format,
                &mut buffer,
            )?;
        }
        ProjectsCommand::Delete(args) => {
            let response = client
                .delete_project(&args.project_id)
                .await
                .with_context(|| format!("failed to delete project '{}'", args.project_id))?;
            write_delete_summary(context.output_format, &response, &mut buffer)?;
        }
        ProjectsCommand::Statuses(args) => {
            let response = client
                .get_project_statuses(&args.project)
                .await
                .with_context(|| {
                    format!("failed to fetch statuses for project '{}'", args.project)
                })?;
            render(
                ProjectStatuses(&response),
                context.output_format,
                &mut buffer,
            )?;
        }
        ProjectsCommand::Status { command } => match command {
            StatusCommand::Update(args) => {
                let record = update_status(client, args).await?;
                render(
                    ProjectRecords(&[record]),
                    context.output_format,
                    &mut buffer,
                )?;
            }
        },
    }
    write_stdout(&buffer)?;
    Ok(())
}

async fn create_project(
    client: &dyn HydraClientInterface,
    args: CreateProjectArgs,
) -> Result<ProjectRecord> {
    let body = load_body_file(&args.body_file)?;
    let creator = resolve_username(client).await?;
    let project = Project::new(
        args.key,
        args.name,
        body.statuses,
        body.default_status_key,
        creator,
        false,
    );
    let request = UpsertProjectRequest::new(project.clone());
    let response = client
        .create_project(&request)
        .await
        .context("failed to create project")?;
    Ok(ProjectRecord::new(
        response.project_id,
        response.version,
        project,
    ))
}

async fn update_project(
    client: &dyn HydraClientInterface,
    args: UpdateProjectArgs,
) -> Result<ProjectRecord> {
    let current = client
        .get_project(&args.project_id)
        .await
        .with_context(|| format!("failed to fetch project '{}'", args.project_id))?;

    let (statuses, default_status_key) = if let Some(path) = args.body_file.as_ref() {
        let body = load_body_file(path)?;
        (body.statuses, body.default_status_key)
    } else {
        (
            current.project.statuses.clone(),
            current.project.default_status_key.clone(),
        )
    };

    let prompt_path = apply_prompt_path_arg(args.prompt_path, current.project.prompt_path.clone());

    let mut project = Project::new(
        args.key.unwrap_or(current.project.key),
        args.name.unwrap_or(current.project.name),
        statuses,
        default_status_key,
        current.project.creator,
        current.project.deleted,
    );
    project.prompt_path = prompt_path;

    let request = UpsertProjectRequest::new(project.clone());
    let response = client
        .update_project(&args.project_id, &request)
        .await
        .with_context(|| format!("failed to update project '{}'", args.project_id))?;
    Ok(ProjectRecord::new(
        response.project_id,
        response.version,
        project,
    ))
}

async fn update_status(
    client: &dyn HydraClientInterface,
    args: UpdateStatusArgs,
) -> Result<ProjectRecord> {
    let current = client
        .get_project(&args.project_id)
        .await
        .with_context(|| format!("failed to fetch project '{}'", args.project_id))?;

    let mut project = current.project.clone();
    let status = project
        .statuses
        .iter_mut()
        .find(|s| s.key == args.status_key)
        .with_context(|| {
            format!(
                "project '{}' has no status with key '{}'",
                args.project_id, args.status_key
            )
        })?;

    status.prompt_path = apply_prompt_path_arg(args.prompt_path, status.prompt_path.clone());

    let request = UpsertProjectRequest::new(project.clone());
    let response = client
        .update_project(&args.project_id, &request)
        .await
        .with_context(|| format!("failed to update project '{}'", args.project_id))?;
    Ok(ProjectRecord::new(
        response.project_id,
        response.version,
        project,
    ))
}

/// Map a `--prompt-path` CLI value onto the resulting `Option<String>`:
/// absent (`None`) keeps the existing value, an explicit empty string
/// clears it to `None`, and a non-empty string sets it.
fn apply_prompt_path_arg(arg: Option<String>, current: Option<String>) -> Option<String> {
    match arg {
        None => current,
        Some(value) if value.is_empty() => None,
        Some(value) => Some(value),
    }
}

fn write_delete_summary<W: std::io::Write>(
    format: ResolvedOutputFormat,
    response: &UpsertProjectResponse,
    writer: &mut W,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Pretty => {
            writeln!(
                writer,
                "Deleted project '{}' (version {})",
                response.project_id, response.version
            )?;
        }
        ResolvedOutputFormat::Jsonl => {
            serde_json::to_writer(&mut *writer, response)?;
            writer.write_all(b"\n")?;
        }
    }
    writer.flush()?;
    Ok(())
}

/// Parse a `--project <id-or-key>` value: try a `ProjectId` first, and if
/// that fails, fall back to treating the input as a `ProjectKey`. Returns
/// an enum so callers can dispatch resolution (key lookups hit the list
/// endpoint).
#[derive(Debug, Clone)]
pub enum ProjectRef {
    Id(ProjectId),
    Key(ProjectKey),
}

impl FromStr for ProjectRef {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if let Ok(id) = ProjectId::try_from(value.to_string()) {
            return Ok(ProjectRef::Id(id));
        }
        ProjectKey::try_new(value)
            .map(ProjectRef::Key)
            .map_err(|err| {
                format!("'{value}' is neither a valid project id nor a valid project key: {err}")
            })
    }
}

impl ProjectRef {
    /// Resolve to a `ProjectId` by listing projects when given a key.
    pub async fn resolve(&self, client: &dyn HydraClientInterface) -> Result<ProjectId> {
        match self {
            ProjectRef::Id(id) => Ok(id.clone()),
            ProjectRef::Key(key) => {
                let projects = client
                    .list_projects()
                    .await
                    .context("failed to list projects to resolve project key")?
                    .projects;
                projects
                    .into_iter()
                    .find(|record| &record.project.key == key)
                    .map(|record| record.project_id)
                    .with_context(|| format!("no project found with key '{key}'"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_ref_parses_id() {
        let r = ProjectRef::from_str("j-abcdef").unwrap();
        match r {
            ProjectRef::Id(_) => {}
            ProjectRef::Key(_) => panic!("expected an id"),
        }
    }

    #[test]
    fn project_ref_parses_key() {
        let r = ProjectRef::from_str("engineering").unwrap();
        match r {
            ProjectRef::Key(key) => assert_eq!(key.as_str(), "engineering"),
            ProjectRef::Id(_) => panic!("expected a key"),
        }
    }

    #[test]
    fn project_ref_rejects_invalid_token() {
        let err = ProjectRef::from_str("Bad Value!").unwrap_err();
        assert!(err.contains("neither a valid project id nor a valid project key"));
    }

    #[test]
    fn apply_prompt_path_arg_none_keeps_current() {
        assert_eq!(
            apply_prompt_path_arg(None, Some("/a".into())),
            Some("/a".into())
        );
        assert_eq!(apply_prompt_path_arg(None, None), None);
    }

    #[test]
    fn apply_prompt_path_arg_empty_clears() {
        assert_eq!(
            apply_prompt_path_arg(Some("".into()), Some("/a".into())),
            None
        );
        assert_eq!(apply_prompt_path_arg(Some("".into()), None), None);
    }

    #[test]
    fn apply_prompt_path_arg_some_overrides() {
        assert_eq!(
            apply_prompt_path_arg(Some("/b".into()), Some("/a".into())),
            Some("/b".into())
        );
        assert_eq!(
            apply_prompt_path_arg(Some("/b".into()), None),
            Some("/b".into())
        );
    }
}
