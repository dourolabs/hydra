use crate::{
    client::HydraClientInterface,
    command::{
        output::{render, CommandContext, ProjectRecords, ProjectStatuses, ResolvedOutputFormat},
        project_body_file::load_body_file,
        utils::resolve_username,
    },
    output_writer::write_stdout,
};
use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use hydra_common::api::v1::projects::{
    Project, ProjectIdOrDefault, ProjectKey, ProjectRecord, RenameStatusRequest, StatusDefinition,
    StatusKey, UpsertProjectRequest, UpsertProjectResponse,
};
use hydra_common::ProjectId;
use std::path::{Path, PathBuf};
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
    /// seeded default project's statuses.
    Statuses(StatusesProjectArgs),
    /// Operate on a single status within a project.
    Status {
        #[command(subcommand)]
        command: StatusCommand,
    },
    /// Write a richly-commented sample project body file to disk. The
    /// output is a valid `--body-file` input for `projects create` /
    /// `projects update` and is the documented starting point for
    /// authoring a new project.
    SampleConfig(SampleConfigArgs),
}

#[derive(Debug, Subcommand)]
pub enum StatusCommand {
    /// Update fields on a single status within a project.
    Update(UpdateStatusArgs),
    /// Rename a status key in place. Existing issues are not orphaned
    /// (their `status_sequence` storage identity is preserved) and read
    /// back as the new key.
    Rename(RenameStatusArgs),
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
    /// `statuses` list (the project-specific set of issue statuses).
    #[arg(long = "body-file", value_name = "PATH")]
    pub body_file: PathBuf,
}

#[derive(Debug, Clone, Args)]
pub struct GetProjectArgs {
    /// Project id (e.g. `j-abc123`).
    #[arg(value_name = "PROJECT_ID")]
    pub project_id: ProjectId,

    /// Emit the project's body in `--body-file` YAML shape (statuses)
    /// on stdout, suitable for piping back into
    /// `projects update --body-file -`. Overrides the default pretty /
    /// jsonl rendering.
    #[arg(long = "body-yaml")]
    pub body_yaml: bool,
}

#[derive(Debug, Clone, Args)]
pub struct SampleConfigArgs {
    /// Destination path for the sample body YAML.
    #[arg(value_name = "OUTPUT_PATH")]
    pub output_path: PathBuf,

    /// Overwrite `<OUTPUT_PATH>` if it already exists. Without this flag
    /// the command refuses to clobber an existing file.
    #[arg(long)]
    pub force: bool,
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

    /// Path to a JSON or YAML file containing the new body (`statuses`
    /// list). Defaults to the existing body.
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
pub struct RenameStatusArgs {
    /// Project id whose status is being renamed.
    #[arg(value_name = "PROJECT_ID")]
    pub project_id: ProjectId,

    /// Current status key.
    #[arg(value_name = "FROM_KEY")]
    pub from: StatusKey,

    /// New status key.
    #[arg(value_name = "TO_KEY")]
    pub to: StatusKey,
}

#[derive(Debug, Clone, Args)]
pub struct DeleteProjectArgs {
    /// Project id to delete.
    #[arg(value_name = "PROJECT_ID")]
    pub project_id: ProjectId,
}

#[derive(Debug, Clone, Args)]
pub struct StatusesProjectArgs {
    /// Project id or the literal `default` for the seeded default
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
            if args.body_yaml {
                let yaml = render_body_yaml(&record.project)?;
                buffer.extend_from_slice(yaml.as_bytes());
            } else {
                render(
                    ProjectRecords(&[record]),
                    context.output_format,
                    &mut buffer,
                )?;
            }
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
            StatusCommand::Rename(args) => {
                let record = rename_status(client, args).await?;
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
    let body = load_body_file(&args.body_file)?;
    let creator = resolve_username(client).await?;
    let project = Project::new(args.key, args.name, body.statuses, creator, false, 0.0);
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

    let statuses = if let Some(path) = args.body_file.as_ref() {
        let body = load_body_file(path)?;
        body.statuses
    } else {
        current.project.statuses.clone()
    };

    let prompt_path = apply_prompt_path_arg(args.prompt_path, current.project.prompt_path.clone());

    let mut project = Project::new(
        args.key.unwrap_or(current.project.key),
        args.name.unwrap_or(current.project.name),
        statuses,
        current.project.creator,
        current.project.deleted,
        current.project.priority,
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

async fn rename_status(
    client: &dyn HydraClientInterface,
    args: RenameStatusArgs,
) -> Result<ProjectRecord> {
    let current = client
        .get_project(&args.project_id)
        .await
        .with_context(|| format!("failed to fetch project '{}'", args.project_id))?;

    let request = RenameStatusRequest::new(args.from.clone(), args.to.clone());
    let response = client
        .rename_project_status(&args.project_id, &request)
        .await
        .with_context(|| {
            format!(
                "failed to rename status '{}' to '{}' on project '{}'",
                args.from, args.to, args.project_id,
            )
        })?;

    let mut project = current.project;
    for status in project.statuses.iter_mut() {
        if status.key == args.from {
            status.key = args.to.clone();
            break;
        }
    }
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

/// Render the body-file slice (`statuses`) of a project as YAML. This
/// is the inverse of [`load_body_file`] for the statuses field: piping
/// the output back through `projects update --body-file` is a no-op for
/// the body slice (modulo whitespace/comment loss).
///
/// `ProjectBodyFile` itself only derives `Deserialize`; we keep it that
/// way and serialize a borrowed view here so the parser stays decoupled
/// from the writer.
fn render_body_yaml(project: &Project) -> Result<String> {
    #[derive(serde::Serialize)]
    struct BodyView<'a> {
        statuses: &'a [StatusDefinition],
    }
    let view = BodyView {
        statuses: &project.statuses,
    };
    serde_yaml_ng::to_string(&view).context("failed to serialize project body as YAML")
}

fn write_sample_config(path: &Path, force: bool) -> Result<()> {
    if path.exists() && !force {
        bail!(
            "refusing to overwrite existing file '{}' (pass --force to overwrite)",
            path.display()
        );
    }
    std::fs::write(path, SAMPLE_PROJECT_BODY_YAML.as_bytes())
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
            writeln!(writer, "Wrote sample project body to '{}'", path.display())?;
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

/// Richly-commented sample project body file. Round-trips through
/// [`load_body_file`] (see `sample_project_body_yaml_round_trips` below)
/// and is the documented starting point for `--body-file` authoring.
const SAMPLE_PROJECT_BODY_YAML: &str = include_str!("sample_project_body.yaml");

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

    /// Parse the bundled sample body file and assert it covers every
    /// surface call-outs in the issue: ≥3 statuses, an `on_enter.assign_to`,
    /// an `on_enter.attach_form`, a `prompt_path`, an `interactive: true`,
    /// and varied dependency-graph booleans. Without this we'd silently
    /// ship a sample that no longer parses (or quietly drifts off the
    /// documented surface).
    #[test]
    fn sample_project_body_yaml_round_trips() {
        let body = serde_yaml_ng::from_str::<crate::command::project_body_file::ProjectBodyFile>(
            SAMPLE_PROJECT_BODY_YAML,
        )
        .expect("sample yaml must parse as a ProjectBodyFile");
        assert!(
            body.statuses.len() >= 3,
            "sample must exercise ≥3 statuses, got {}",
            body.statuses.len(),
        );
        assert!(
            body.statuses.iter().any(|s| s
                .on_enter
                .as_ref()
                .and_then(|e| e.assign_to.as_ref())
                .is_some()),
            "sample must exercise on_enter.assign_to",
        );
        assert!(
            body.statuses.iter().any(|s| s
                .on_enter
                .as_ref()
                .and_then(|e| e.attach_form.as_ref())
                .is_some()),
            "sample must exercise on_enter.attach_form",
        );
        assert!(
            body.statuses.iter().any(|s| s.prompt_path.is_some()),
            "sample must exercise prompt_path",
        );
        assert!(
            body.statuses.iter().any(|s| s.interactive),
            "sample must exercise interactive: true",
        );
        assert!(
            body.statuses.iter().any(|s| s.unblocks_parents),
            "sample must exercise unblocks_parents: true",
        );
        assert!(
            body.statuses.iter().any(|s| s.unblocks_dependents),
            "sample must exercise unblocks_dependents: true",
        );
        assert!(
            body.statuses.iter().any(|s| s.cascades_to_children),
            "sample must exercise cascades_to_children: true",
        );
    }

    /// The sample is shipped as the documented `--body-file` starting point,
    /// so the constant SHOULD contain inline `#` comments — a regression
    /// where someone replaced it with `serde_yaml_ng::to_string(...)` (which
    /// drops comments) would silently degrade the UX.
    #[test]
    fn sample_project_body_yaml_contains_inline_comments() {
        assert!(
            SAMPLE_PROJECT_BODY_YAML.contains('#'),
            "sample yaml lost its inline `#` comments",
        );
    }

    fn build_project_fixture() -> Project {
        use hydra_common::agents::AgentName;
        use hydra_common::api::v1::projects::StatusOnEnter;
        use hydra_common::api::v1::users::Username;
        use hydra_common::principal::Principal;

        let mut backlog = StatusDefinition::new(
            StatusKey::try_new("backlog").unwrap(),
            "Backlog".to_string(),
            "#9b59b6".parse().unwrap(),
            false,
            false,
            false,
            Some(StatusOnEnter::new(
                Some(Principal::agent(AgentName::try_new("pm").unwrap())),
                None,
            )),
        );
        backlog.prompt_path = Some("/projects/fixture/statuses/backlog.md".into());
        backlog.interactive = true;

        let pending_release = StatusDefinition::new(
            StatusKey::try_new("pending-release").unwrap(),
            "Pending release".to_string(),
            "#2ecc71".parse().unwrap(),
            true,
            true,
            false,
            None,
        );

        Project::new(
            ProjectKey::try_new("fixture").unwrap(),
            "Fixture".to_string(),
            vec![backlog, pending_release],
            Username::try_new("jayantk").unwrap(),
            false,
            0.0,
        )
    }

    /// Render an in-memory `Project` via `render_body_yaml`, parse the
    /// output back through the same `load_body_file` parser the CLI uses,
    /// and assert the body slice survives unchanged. Backs the `get --config`
    /// → `update --body-file` round-trip without needing a live server.
    #[test]
    fn render_body_yaml_round_trips_through_body_file_parser() {
        let project = build_project_fixture();
        let yaml = render_body_yaml(&project).unwrap();
        let body =
            serde_yaml_ng::from_str::<crate::command::project_body_file::ProjectBodyFile>(&yaml)
                .expect("rendered yaml must parse as a ProjectBodyFile");
        assert_eq!(body.statuses, project.statuses);
    }

    /// The body view must NOT leak project-level fields that `ProjectBodyFile`
    /// doesn't carry (`key`, `name`, `creator`, `deleted`, project-level
    /// `prompt_path`). The body-file parser is tolerant of unknown fields,
    /// so a future refactor that accidentally serialized the whole `Project`
    /// would still appear to "round-trip" while quietly polluting the file
    /// the user copies around as their config.
    #[test]
    fn render_body_yaml_omits_project_envelope_fields() {
        let mut project = build_project_fixture();
        project.prompt_path = Some("/projects/fixture/prompt.md".into());
        let yaml = render_body_yaml(&project).unwrap();
        let value: serde_yaml_ng::Value =
            serde_yaml_ng::from_str(&yaml).expect("rendered yaml must parse");
        let mapping = value
            .as_mapping()
            .expect("rendered body should be a YAML mapping");
        let keys: std::collections::BTreeSet<_> = mapping
            .keys()
            .filter_map(|k| k.as_str().map(str::to_owned))
            .collect();
        let expected: std::collections::BTreeSet<String> =
            ["statuses"].iter().map(|s| s.to_string()).collect();
        assert_eq!(
            keys, expected,
            "top-level body keys should be exactly statuses; got {keys:?}",
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
        // File must be left untouched.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "existing\n");
    }

    #[test]
    fn write_sample_config_overwrites_with_force() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.yaml");
        std::fs::write(&path, b"existing\n").unwrap();
        write_sample_config(&path, true).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents, SAMPLE_PROJECT_BODY_YAML);
    }

    #[test]
    fn write_sample_config_writes_to_new_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.yaml");
        write_sample_config(&path, false).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents, SAMPLE_PROJECT_BODY_YAML);
    }

    #[test]
    fn get_project_args_parses_body_yaml_flag() {
        use clap::Parser;

        #[derive(Debug, Parser)]
        struct Cli {
            #[command(flatten)]
            args: GetProjectArgs,
        }

        let cli = Cli::try_parse_from(["cli", "j-abcdef", "--body-yaml"]).expect("parse");
        assert!(cli.args.body_yaml);

        let cli = Cli::try_parse_from(["cli", "j-abcdef"]).expect("parse");
        assert!(!cli.args.body_yaml);
    }

    /// Drive the full top-level `Cli` parser end-to-end to catch flag
    /// collisions with the global `--config <FILE>`. Parsing
    /// `GetProjectArgs` in isolation can't see the global flag, so a
    /// subcommand-level flag that shadows it would only surface as a
    /// runtime panic. Keep this test next to the flag definition so the
    /// next person who reaches for `--config` (or any other global-flag
    /// name) discovers the conflict at `cargo test` time.
    #[test]
    fn cli_parses_projects_get_with_body_yaml_flag() {
        use crate::cli::{Cli, Commands};
        use clap::Parser;

        let cli = Cli::try_parse_from(["hydra", "projects", "get", "j-abcdef", "--body-yaml"])
            .expect("parse");
        match cli.command {
            Some(Commands::Projects {
                command: ProjectsCommand::Get(args),
            }) => {
                assert_eq!(args.project_id.to_string(), "j-abcdef");
                assert!(args.body_yaml);
            }
            _ => panic!("expected `projects get` subcommand"),
        }
    }

    #[test]
    fn sample_config_args_parses_force_flag() {
        use clap::Parser;

        #[derive(Debug, Parser)]
        struct Cli {
            #[command(flatten)]
            args: SampleConfigArgs,
        }

        let cli = Cli::try_parse_from(["cli", "/tmp/x.yaml"]).expect("parse");
        assert_eq!(cli.args.output_path, PathBuf::from("/tmp/x.yaml"));
        assert!(!cli.args.force);

        let cli = Cli::try_parse_from(["cli", "/tmp/x.yaml", "--force"]).expect("parse");
        assert!(cli.args.force);

        // Missing positional arg should error.
        assert!(Cli::try_parse_from(["cli"]).is_err());
    }
}
