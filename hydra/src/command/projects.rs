use crate::{
    client::HydraClientInterface,
    command::{
        output::{render, CommandContext, ProjectRecords, ProjectStatuses, ResolvedOutputFormat},
        utils::resolve_username,
    },
    output_writer::write_stdout,
};
use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use hydra_common::api::v1::projects::{
    Project, ProjectIdOrDefault, ProjectKey, ProjectRecord, StatusKey, UpsertProjectRequest,
    UpsertProjectResponse,
};
use hydra_common::ProjectId;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Body file payload for `projects create` / `projects update`. Describes a
/// project's status list and its `default_status_key`. The CLI fills in the
/// `key`, `name`, and `creator` fields on top of this.
#[derive(Debug, serde::Deserialize)]
struct ProjectBodyFile {
    #[serde(default)]
    statuses: Vec<hydra_common::api::v1::projects::StatusDefinition>,
    default_status_key: StatusKey,
}

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

    let project = Project::new(
        args.key.unwrap_or(current.project.key),
        args.name.unwrap_or(current.project.name),
        statuses,
        default_status_key,
        current.project.creator,
        current.project.deleted,
    );

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

fn load_body_file(path: &Path) -> Result<ProjectBodyFile> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read project body file '{}'", path.display()))?;
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        bail!("project body file '{}' is empty", path.display());
    }
    if let Ok(body) = serde_json::from_str::<ProjectBodyFile>(trimmed) {
        return Ok(body);
    }
    serde_yaml_ng::from_str::<ProjectBodyFile>(trimmed)
        .with_context(|| format!("failed to parse project body file '{}'", path.display()))
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
    use hydra_common::api::v1::projects::{IconKey, StatusDefinition};
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_body(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file
    }

    #[test]
    fn load_body_file_parses_json() {
        let file = write_body(
            r##"{
                "statuses": [
                    {
                        "key": "open",
                        "label": "Open",
                        "icon": "circle",
                        "color": "#abcdef",
                        "unblocks_parents": false,
                        "unblocks_dependents": false,
                        "cascades_to_children": false
                    }
                ],
                "default_status_key": "open"
            }"##,
        );
        let body = load_body_file(file.path()).unwrap();
        assert_eq!(body.statuses.len(), 1);
        assert_eq!(body.statuses[0].key, StatusKey::try_new("open").unwrap());
        assert_eq!(body.default_status_key, StatusKey::try_new("open").unwrap());
    }

    #[test]
    fn load_body_file_parses_yaml() {
        let file = write_body(
            r##"
statuses:
  - key: open
    label: Open
    icon: circle
    color: "#abcdef"
    unblocks_parents: false
    unblocks_dependents: false
    cascades_to_children: false
default_status_key: open
"##,
        );
        let body = load_body_file(file.path()).unwrap();
        assert_eq!(body.statuses.len(), 1);
        assert_eq!(body.default_status_key, StatusKey::try_new("open").unwrap());
    }

    #[test]
    fn load_body_file_rejects_empty() {
        let file = write_body("");
        let err = load_body_file(file.path()).unwrap_err();
        assert!(err.to_string().contains("is empty"));
    }

    #[test]
    fn load_body_file_rejects_malformed() {
        let file = write_body("{not valid");
        let err = load_body_file(file.path()).unwrap_err();
        assert!(err.to_string().contains("failed to parse"), "got: {err:?}");
    }

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
    fn status_definition_roundtrips_through_body_file() {
        let def = StatusDefinition::new(
            StatusKey::try_new("inbox").unwrap(),
            "Inbox".into(),
            IconKey::try_new("inbox").unwrap(),
            "#ffaa00".parse().unwrap(),
            false,
            false,
            false,
            None,
        );
        let json = format!(
            r#"{{ "statuses": [{}], "default_status_key": "inbox" }}"#,
            serde_json::to_string(&def).unwrap()
        );
        let file = write_body(&json);
        let body = load_body_file(file.path()).unwrap();
        assert_eq!(body.statuses, vec![def]);
    }
}
