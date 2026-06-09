use crate::{
    client::HydraClientInterface,
    command::{
        output::{render, CommandContext, ProjectRecords, ProjectStatuses, ResolvedOutputFormat},
        project_body_file::load_status_body_file,
        utils::resolve_username,
    },
    output_writer::write_stdout,
};
use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use hydra_common::api::v1::projects::{
    Project, ProjectKey, ProjectRecord, ProjectRef, StatusKey, UpsertProjectRequest,
    UpsertProjectResponse,
};

#[cfg(test)]
use hydra_common::api::v1::projects::StatusDefinition;
use std::path::{Path, PathBuf};

#[derive(Debug, Subcommand)]
pub enum ProjectsCommand {
    /// List configured projects.
    List,
    /// Create a new project. Statuses are managed independently via
    /// `projects status create / update / delete`.
    Create(CreateProjectArgs),
    /// Get a project by its id.
    Get(GetProjectArgs),
    /// Update project-level fields (key, name, prompt path). Statuses
    /// are managed independently via `projects status create / update /
    /// delete`.
    Update(UpdateProjectArgs),
    /// Soft-delete a project.
    Delete(DeleteProjectArgs),
    /// List the status definitions for a project. Pass `default` for the
    /// seeded default project's statuses.
    Statuses(StatusesProjectArgs),
    /// Operate on a single status within a project.
    Status {
        #[command(subcommand)]
        command: StatusCommand,
    },
    /// Write a richly-commented sample status body file to disk. The
    /// output is a valid `--body-file` input for
    /// `projects status create` / `projects status update`.
    SampleConfig(SampleConfigArgs),
}

#[derive(Debug, Subcommand)]
pub enum StatusCommand {
    /// Add a new status to a project. Loads the status definition from
    /// a YAML/JSON `--body-file`.
    Create(CreateStatusArgs),
    /// Update a single status on a project. A body whose `key`
    /// differs from `<status_key>` is a rename — the storage identity
    /// is preserved.
    Update(UpdateStatusArgs),
    /// Delete a status from a project. Fails if any issue still
    /// references the status.
    Delete(DeleteStatusArgs),
}

#[derive(Debug, Clone, Args)]
pub struct CreateProjectArgs {
    /// Project key (unique slug; lowercase letters, digits, and `-`).
    #[arg(long, value_name = "KEY")]
    pub key: ProjectKey,

    /// Human-readable project name.
    #[arg(long, value_name = "NAME")]
    pub name: String,

    /// Doc-store path for the project-layer prompt slice. Optional.
    /// Non-empty values should be absolute doc-store paths starting
    /// with `/` (the server is authoritative).
    #[arg(long = "prompt-path", value_name = "PATH")]
    pub prompt_path: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct GetProjectArgs {
    /// Project id (e.g. `j-abc123`) or key (e.g. `engineering`).
    #[arg(value_name = "PROJECT_ID_OR_KEY")]
    pub project_ref: ProjectRef,
}

#[derive(Debug, Clone, Args)]
pub struct SampleConfigArgs {
    /// Destination path for the sample status body YAML.
    #[arg(value_name = "OUTPUT_PATH")]
    pub output_path: PathBuf,

    /// Overwrite `<OUTPUT_PATH>` if it already exists. Without this flag
    /// the command refuses to clobber an existing file.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Clone, Args)]
pub struct UpdateProjectArgs {
    /// Project id (e.g. `j-abc123`) or key (e.g. `engineering`).
    #[arg(value_name = "PROJECT_ID_OR_KEY")]
    pub project_ref: ProjectRef,

    /// New project key. Defaults to the existing value.
    #[arg(long, value_name = "KEY")]
    pub key: Option<ProjectKey>,

    /// New human-readable name. Defaults to the existing value.
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,

    /// Doc-store path for the project-layer prompt slice. Omit to leave
    /// the existing value unchanged; pass `--prompt-path ""` to clear it.
    /// Non-empty values should be absolute doc-store paths starting with
    /// `/` (the server is authoritative).
    #[arg(long = "prompt-path", value_name = "PATH")]
    pub prompt_path: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct CreateStatusArgs {
    /// Project id (e.g. `j-abc123`) or key (e.g. `engineering`).
    #[arg(value_name = "PROJECT_ID_OR_KEY")]
    pub project_ref: ProjectRef,

    /// Path to a JSON or YAML file containing the `StatusDefinition`
    /// body to add.
    #[arg(long = "body-file", value_name = "PATH")]
    pub body_file: PathBuf,
}

#[derive(Debug, Clone, Args)]
pub struct UpdateStatusArgs {
    /// Project id (e.g. `j-abc123`) or key (e.g. `engineering`).
    #[arg(value_name = "PROJECT_ID_OR_KEY")]
    pub project_ref: ProjectRef,

    /// Status key (within the project) to update. If the body's `key`
    /// field is different, the status is renamed in place.
    #[arg(value_name = "STATUS_KEY")]
    pub status_key: StatusKey,

    /// Path to a JSON or YAML file containing the new `StatusDefinition`
    /// body.
    #[arg(long = "body-file", value_name = "PATH")]
    pub body_file: PathBuf,
}

#[derive(Debug, Clone, Args)]
pub struct DeleteStatusArgs {
    /// Project id (e.g. `j-abc123`) or key (e.g. `engineering`).
    #[arg(value_name = "PROJECT_ID_OR_KEY")]
    pub project_ref: ProjectRef,

    /// Status key (within the project) to delete.
    #[arg(value_name = "STATUS_KEY")]
    pub status_key: StatusKey,
}

#[derive(Debug, Clone, Args)]
pub struct DeleteProjectArgs {
    /// Project id (e.g. `j-abc123`) or key (e.g. `engineering`).
    #[arg(value_name = "PROJECT_ID_OR_KEY")]
    pub project_ref: ProjectRef,
}

#[derive(Debug, Clone, Args)]
pub struct StatusesProjectArgs {
    /// Project id (e.g. `j-abc123`) or key (e.g. `engineering`). Pass
    /// the literal `default` for the seeded default project's statuses.
    #[arg(value_name = "PROJECT_ID_OR_KEY")]
    pub project_ref: ProjectRef,
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
                .get_project(&args.project_ref)
                .await
                .with_context(|| format!("failed to fetch project '{}'", args.project_ref))?;
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
                .delete_project(&args.project_ref)
                .await
                .with_context(|| format!("failed to delete project '{}'", args.project_ref))?;
            write_delete_summary(context.output_format, &response, &mut buffer)?;
        }
        ProjectsCommand::Statuses(args) => {
            let response = client
                .get_project_statuses(&args.project_ref)
                .await
                .with_context(|| {
                    format!(
                        "failed to fetch statuses for project '{}'",
                        args.project_ref
                    )
                })?;
            render(
                ProjectStatuses(&response),
                context.output_format,
                &mut buffer,
            )?;
        }
        ProjectsCommand::Status { command } => match command {
            StatusCommand::Create(args) => {
                let record = create_status(client, args).await?;
                render(
                    ProjectRecords(&[record]),
                    context.output_format,
                    &mut buffer,
                )?;
            }
            StatusCommand::Update(args) => {
                let record = update_status(client, args).await?;
                render(
                    ProjectRecords(&[record]),
                    context.output_format,
                    &mut buffer,
                )?;
            }
            StatusCommand::Delete(args) => {
                let record = delete_status(client, args).await?;
                render(
                    ProjectRecords(&[record]),
                    context.output_format,
                    &mut buffer,
                )?;
            }
        },
        ProjectsCommand::SampleConfig(args) => {
            write_sample_config(&args.output_path, args.force)?;
            write_sample_config_summary(context.output_format, &args.output_path, &mut buffer)?;
        }
    }
    write_stdout(&buffer)?;
    Ok(())
}

async fn create_project(
    client: &dyn HydraClientInterface,
    args: CreateProjectArgs,
) -> Result<ProjectRecord> {
    let creator = resolve_username(client).await?;
    let mut request = UpsertProjectRequest::new(args.key.clone(), args.name.clone());
    request.prompt_path = args.prompt_path.clone();
    let response = client
        .create_project(&request)
        .await
        .context("failed to create project")?;
    let mut project = Project::new(args.key, args.name, Vec::new(), creator, false, 0.0);
    project.prompt_path = args.prompt_path;
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
        .get_project(&args.project_ref)
        .await
        .with_context(|| format!("failed to fetch project '{}'", args.project_ref))?;

    let prompt_path = apply_prompt_path_arg(args.prompt_path, current.project.prompt_path.clone());

    let mut request = UpsertProjectRequest::new(
        args.key.clone().unwrap_or_else(|| current.project.key.clone()),
        args.name.clone().unwrap_or_else(|| current.project.name.clone()),
    );
    request.prompt_path = prompt_path.clone();
    request.priority = current.project.priority;

    let response = client
        .update_project(&args.project_ref, &request)
        .await
        .with_context(|| format!("failed to update project '{}'", args.project_ref))?;

    let mut project = Project::new(
        request.key,
        request.name,
        current.project.statuses,
        current.project.creator,
        current.project.deleted,
        current.project.priority,
    );
    project.prompt_path = prompt_path;
    Ok(ProjectRecord::new(
        response.project_id,
        response.version,
        project,
    ))
}

async fn create_status(
    client: &dyn HydraClientInterface,
    args: CreateStatusArgs,
) -> Result<ProjectRecord> {
    let body = load_status_body_file(&args.body_file)?;
    client
        .create_project_status(&args.project_ref, &body)
        .await
        .with_context(|| {
            format!(
                "failed to add status '{}' to project '{}'",
                body.key, args.project_ref
            )
        })?;
    let record = client
        .get_project(&args.project_ref)
        .await
        .with_context(|| format!("failed to fetch project '{}'", args.project_ref))?;
    Ok(record)
}

async fn update_status(
    client: &dyn HydraClientInterface,
    args: UpdateStatusArgs,
) -> Result<ProjectRecord> {
    let body = load_status_body_file(&args.body_file)?;
    client
        .update_project_status(&args.project_ref, &args.status_key, &body)
        .await
        .with_context(|| {
            format!(
                "failed to update status '{}' on project '{}'",
                args.status_key, args.project_ref
            )
        })?;
    let record = client
        .get_project(&args.project_ref)
        .await
        .with_context(|| format!("failed to fetch project '{}'", args.project_ref))?;
    Ok(record)
}

async fn delete_status(
    client: &dyn HydraClientInterface,
    args: DeleteStatusArgs,
) -> Result<ProjectRecord> {
    client
        .delete_project_status(&args.project_ref, &args.status_key)
        .await
        .with_context(|| {
            format!(
                "failed to delete status '{}' from project '{}'",
                args.status_key, args.project_ref
            )
        })?;
    let record = client
        .get_project(&args.project_ref)
        .await
        .with_context(|| format!("failed to fetch project '{}'", args.project_ref))?;
    Ok(record)
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

fn write_sample_config(path: &Path, force: bool) -> Result<()> {
    if path.exists() && !force {
        bail!(
            "refusing to overwrite existing file '{}' (pass --force to overwrite)",
            path.display()
        );
    }
    std::fs::write(path, SAMPLE_STATUS_BODY_YAML.as_bytes())
        .with_context(|| format!("failed to write sample config to '{}'", path.display()))?;
    Ok(())
}

fn write_sample_config_summary<W: std::io::Write>(
    format: ResolvedOutputFormat,
    path: &Path,
    writer: &mut W,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Pretty => {
            writeln!(writer, "Wrote sample status body to '{}'", path.display())?;
        }
        ResolvedOutputFormat::Jsonl => {
            let line = serde_json::json!({ "output_path": path.display().to_string() });
            serde_json::to_writer(&mut *writer, &line)?;
            writer.write_all(b"\n")?;
        }
    }
    writer.flush()?;
    Ok(())
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

/// Richly-commented sample status body file. Round-trips through
/// [`load_status_body_file`] and is the documented starting point for
/// `--body-file` authoring.
const SAMPLE_STATUS_BODY_YAML: &str = include_str!("sample_status_body.yaml");

#[cfg(test)]
mod tests {
    use super::*;

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

    /// The sample yaml must round-trip through [`load_status_body_file`].
    #[test]
    fn sample_status_body_yaml_round_trips() {
        let body = load_sample_body();
        // basic invariants: key + label + color populate, the file
        // exercises the optional fields too.
        assert!(!body.label.is_empty());
        assert!(body.prompt_path.is_some());
    }

    fn load_sample_body() -> StatusDefinition {
        crate::command::project_body_file::parse_status_body(SAMPLE_STATUS_BODY_YAML)
            .expect("sample yaml must parse as a StatusDefinition")
    }

    #[test]
    fn sample_status_body_yaml_contains_inline_comments() {
        assert!(
            SAMPLE_STATUS_BODY_YAML.contains('#'),
            "sample yaml lost its inline `#` comments",
        );
    }

    #[test]
    fn write_sample_config_refuses_to_overwrite_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.yaml");
        std::fs::write(&path, b"existing\n").unwrap();
        let err = write_sample_config(&path, false).unwrap_err();
        assert!(
            err.to_string().contains("refusing to overwrite"),
            "got: {err}",
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "existing\n");
    }

    #[test]
    fn write_sample_config_overwrites_with_force() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.yaml");
        std::fs::write(&path, b"existing\n").unwrap();
        write_sample_config(&path, true).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents, SAMPLE_STATUS_BODY_YAML);
    }

    #[test]
    fn write_sample_config_writes_to_new_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.yaml");
        write_sample_config(&path, false).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents, SAMPLE_STATUS_BODY_YAML);
    }
}
