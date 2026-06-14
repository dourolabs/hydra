use crate::{
    client::HydraClientInterface,
    command::{
        output::{render, CommandContext, ProjectRecords, ProjectStatuses, ResolvedOutputFormat},
        utils::resolve_username,
    },
    output_writer::write_stdout,
};
use anyhow::{anyhow, bail, Context, Result};
use clap::{builder::BoolishValueParser, Args, Subcommand};
use hydra_common::api::v1::issues::SessionSettings;
use hydra_common::api::v1::projects::{
    Project, ProjectKey, ProjectRecord, ProjectRef, StatusDefinition, StatusKey, StatusOnEnter,
    UpsertProjectRequest, UpsertProjectResponse,
};
use hydra_common::api::v1::timeout::Timeout;
use hydra_common::{DocumentPath, Principal, Rgb};
use std::str::FromStr;

#[derive(Debug, Subcommand)]
pub enum ProjectsCommand {
    /// List configured projects.
    List,
    /// Create a new project. Statuses are managed independently via
    /// `projects status create / update / archive`.
    Create(CreateProjectArgs),
    /// Get a project by its id.
    Get(GetProjectArgs),
    /// Update project-level fields (key, name, prompt path). Statuses
    /// are managed independently via `projects status create / update
    /// / archive`.
    Update(UpdateProjectArgs),
    /// Archive a project, cascade-archiving every non-archived issue
    /// it owns. Idempotent.
    Archive(ArchiveProjectArgs),
    /// Unarchive a project. No reverse cascade — previously cascade-
    /// archived issues stay archived.
    Unarchive(ArchiveProjectArgs),
    /// List the status definitions for a project. Pass `default` for the
    /// seeded default project's statuses.
    Statuses(StatusesProjectArgs),
    /// Operate on a single status within a project.
    Status {
        #[command(subcommand)]
        command: StatusCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum StatusCommand {
    /// Add a new status to a project, specified via the direct
    /// `--key/--label/--color/...` flags.
    Create(Box<CreateStatusArgs>),
    /// Update a single status on a project. The direct flags are
    /// overlaid on the current definition; the `--key` flag renames the
    /// status in place (storage identity preserved).
    Update(Box<UpdateStatusArgs>),
    /// Archive a status in place and cascade-archive every non-
    /// archived issue currently at that status. Idempotent.
    Archive(ArchiveStatusArgs),
    /// Unarchive a status. No reverse cascade.
    Unarchive(ArchiveStatusArgs),
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

    /// Per-project container CPU limit override (e.g. `500m`, `2`). Wins
    /// over the global default during spawn; status- and issue-level
    /// `cpu_limit` still win over this.
    #[arg(long = "cpu-limit", value_name = "STRING")]
    pub cpu_limit: Option<String>,

    /// Per-project container memory limit override (e.g. `1Gi`, `512Mi`).
    /// Wins over the global default during spawn; status- and issue-level
    /// `memory_limit` still win over this.
    #[arg(long = "memory-limit", value_name = "STRING")]
    pub memory_limit: Option<String>,

    /// Per-project container image override. Wins over the global default
    /// during spawn; status- and issue-level `image` still win over this.
    #[arg(long = "image", value_name = "STRING")]
    pub image: Option<String>,

    /// Per-project model override. Wins over the global default during
    /// spawn; status- and issue-level `model` still win over this.
    #[arg(long = "model", value_name = "STRING")]
    pub model: Option<String>,

    /// Per-project max-retries override (headless sessions). Wins over
    /// the global default; status- and issue-level `max_retries` still
    /// win over this.
    #[arg(long = "max-retries", value_name = "COUNT")]
    pub max_retries: Option<u32>,

    /// Per-project idle-timeout override (interactive sessions). Pass
    /// `infinite` to disable the worker's idle timeout, or a duration
    /// like `30m` / `2h`. Wins over the global default; status- and
    /// issue-level `idle_timeout` still win over this.
    #[arg(long = "idle-timeout", value_name = "STRING")]
    pub idle_timeout: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct GetProjectArgs {
    /// Project id (e.g. `j-abc123`) or key (e.g. `engineering`).
    #[arg(value_name = "PROJECT_ID_OR_KEY")]
    pub project_ref: ProjectRef,
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

    /// Set the per-project container CPU limit (e.g. `500m`, `2`). Pass
    /// `--cpu-limit ""` to clear the override.
    #[arg(
        long = "cpu-limit",
        value_name = "STRING",
        conflicts_with = "clear_session_settings"
    )]
    pub cpu_limit: Option<String>,

    /// Set the per-project container memory limit (e.g. `1Gi`, `512Mi`).
    /// Pass `--memory-limit ""` to clear the override.
    #[arg(
        long = "memory-limit",
        value_name = "STRING",
        conflicts_with = "clear_session_settings"
    )]
    pub memory_limit: Option<String>,

    /// Set the per-project container image override. Pass `--image ""`
    /// to clear the override.
    #[arg(
        long = "image",
        value_name = "STRING",
        conflicts_with = "clear_session_settings"
    )]
    pub image: Option<String>,

    /// Set the per-project model override. Pass `--model ""` to clear
    /// the override.
    #[arg(
        long = "model",
        value_name = "STRING",
        conflicts_with = "clear_session_settings"
    )]
    pub model: Option<String>,

    /// Set the per-project max-retries override (headless sessions).
    /// Mutually exclusive with `--clear-max-retries`.
    #[arg(
        long = "max-retries",
        value_name = "COUNT",
        conflicts_with_all = ["clear_max_retries", "clear_session_settings"]
    )]
    pub max_retries: Option<u32>,

    /// Clear the per-project max-retries override.
    #[arg(long = "clear-max-retries", conflicts_with = "clear_session_settings")]
    pub clear_max_retries: bool,

    /// Set the per-project idle-timeout override (interactive sessions).
    /// Accepts `infinite` or a positive whole number of seconds.
    /// Mutually exclusive with `--clear-idle-timeout`.
    #[arg(
        long = "idle-timeout",
        value_name = "STRING",
        conflicts_with_all = ["clear_idle_timeout", "clear_session_settings"]
    )]
    pub idle_timeout: Option<String>,

    /// Clear the per-project idle-timeout override.
    #[arg(long = "clear-idle-timeout", conflicts_with = "clear_session_settings")]
    pub clear_idle_timeout: bool,

    /// Drop the project's `session_settings` back to default — wipes
    /// every per-project override at once. Mutually exclusive with the
    /// per-field setters/clearers.
    #[arg(long = "clear-session-settings")]
    pub clear_session_settings: bool,
}

#[derive(Debug, Clone, Args)]
pub struct CreateStatusArgs {
    /// Project id (e.g. `j-abc123`) or key (e.g. `engineering`).
    #[arg(value_name = "PROJECT_ID_OR_KEY")]
    pub project_ref: ProjectRef,

    /// Status key (slug; unique within the project).
    #[arg(long, value_name = "STATUS_KEY")]
    pub key: StatusKey,

    /// Human-readable status label.
    #[arg(long, value_name = "STRING")]
    pub label: String,

    /// Display color as `#RRGGBB`.
    #[arg(long, value_name = "#RRGGBB")]
    pub color: Rgb,

    /// Closing this status unblocks the issue's parents.
    #[arg(long = "unblocks-parents")]
    pub unblocks_parents: bool,

    /// Closing this status unblocks the issue's dependents.
    #[arg(long = "unblocks-dependents")]
    pub unblocks_dependents: bool,

    /// Cascade this status onto the issue's children.
    #[arg(long = "cascades-to-children")]
    pub cascades_to_children: bool,

    /// Issues that land in this status spawn an interactive conversation
    /// instead of a headless session.
    #[arg(long)]
    pub interactive: bool,

    /// Issues that land in this status do not spawn agent sessions —
    /// useful for "tracked but inert" terminal-ish statuses.
    #[arg(long = "suppress-sessions")]
    pub suppress_sessions: bool,

    /// Sort key within the project (smaller = earlier). Defaults to
    /// `0.0` if omitted.
    #[arg(long, value_name = "FLOAT")]
    pub position: Option<f64>,

    /// Auto-archive issues that have sat in this status for at least
    /// this many seconds. Omit to leave the feature off.
    #[arg(long = "auto-archive-after-seconds", value_name = "SECONDS")]
    pub auto_archive_after_seconds: Option<i64>,

    /// Cap on the number of simultaneously-active sessions
    /// (interactive + headless, across all agents) for issues in this
    /// status. Omit to leave the cap off.
    #[arg(long = "max-simultaneous-sessions", value_name = "COUNT")]
    pub max_simultaneous_sessions: Option<u32>,

    /// Per-status container CPU limit override (e.g. `500m`, `2`). Wins
    /// over the global default during spawn; an issue-level
    /// `cpu_limit` still wins over this.
    #[arg(long = "cpu-limit", value_name = "STRING")]
    pub cpu_limit: Option<String>,

    /// Per-status container memory limit override (e.g. `1Gi`, `512Mi`).
    /// Wins over the global default during spawn; an issue-level
    /// `memory_limit` still wins over this.
    #[arg(long = "memory-limit", value_name = "STRING")]
    pub memory_limit: Option<String>,

    /// Doc-store path for the per-status prompt slice.
    #[arg(long = "prompt-path", value_name = "DOC_PATH")]
    pub prompt_path: Option<String>,

    /// On-enter: set the issue's assignee to this principal. Accepts the
    /// canonical path form: `users/<name>`, `agents/<name>`, or
    /// `external/<system>/<name>`.
    #[arg(
        long = "on-enter-assign-to",
        value_name = "PRINCIPAL",
        value_parser = parse_principal_arg,
        conflicts_with = "on_enter_clear_assignee",
    )]
    pub on_enter_assign_to: Option<Principal>,

    /// On-enter: attach this form to the issue (doc-store path).
    #[arg(long = "on-enter-attach-form", value_name = "DOC_PATH")]
    pub on_enter_attach_form: Option<DocumentPath>,

    /// On-enter: unset the issue's assignee. Mutually exclusive with
    /// `--on-enter-assign-to`.
    #[arg(long = "on-enter-clear-assignee")]
    pub on_enter_clear_assignee: bool,

    /// On-enter: tear down agent work attached to the issue — kill any
    /// `Created`/`Pending`/`Running` sessions and close any non-`Closed`
    /// conversations spawned from the issue.
    #[arg(long = "on-enter-teardown-work")]
    pub on_enter_teardown_work: bool,
}

#[derive(Debug, Clone, Args)]
pub struct UpdateStatusArgs {
    /// Project id (e.g. `j-abc123`) or key (e.g. `engineering`).
    #[arg(value_name = "PROJECT_ID_OR_KEY")]
    pub project_ref: ProjectRef,

    /// Status key (within the project) to update. If `--key` is passed,
    /// the status is renamed in place — storage identity is preserved.
    #[arg(value_name = "STATUS_KEY")]
    pub status_key: StatusKey,

    /// Rename the status to this key. Storage identity is preserved.
    #[arg(long, value_name = "STATUS_KEY")]
    pub key: Option<StatusKey>,

    /// Set the human-readable label.
    #[arg(long, value_name = "STRING")]
    pub label: Option<String>,

    /// Set the display color as `#RRGGBB`.
    #[arg(long, value_name = "#RRGGBB")]
    pub color: Option<Rgb>,

    /// Set the doc-store path for the per-status prompt slice. Pass
    /// `--prompt-path ""` to clear it.
    #[arg(long = "prompt-path", value_name = "DOC_PATH")]
    pub prompt_path: Option<String>,

    /// Set `unblocks_parents`. Explicit value form is required
    /// (`--unblocks-parents=true|false`) so omission preserves the
    /// existing value.
    #[arg(
        long = "unblocks-parents",
        value_name = "BOOL",
        num_args = 1,
        value_parser = BoolishValueParser::new(),
    )]
    pub unblocks_parents: Option<bool>,

    /// Set `unblocks_dependents`. Explicit value form is required.
    #[arg(
        long = "unblocks-dependents",
        value_name = "BOOL",
        num_args = 1,
        value_parser = BoolishValueParser::new(),
    )]
    pub unblocks_dependents: Option<bool>,

    /// Set `cascades_to_children`. Explicit value form is required.
    #[arg(
        long = "cascades-to-children",
        value_name = "BOOL",
        num_args = 1,
        value_parser = BoolishValueParser::new(),
    )]
    pub cascades_to_children: Option<bool>,

    /// Set `interactive`. Explicit value form is required.
    #[arg(
        long,
        value_name = "BOOL",
        num_args = 1,
        value_parser = BoolishValueParser::new(),
    )]
    pub interactive: Option<bool>,

    /// Set `suppress_sessions`. Explicit value form is required.
    #[arg(
        long = "suppress-sessions",
        value_name = "BOOL",
        num_args = 1,
        value_parser = BoolishValueParser::new(),
    )]
    pub suppress_sessions: Option<bool>,

    /// Set the sort key within the project.
    #[arg(long, value_name = "FLOAT")]
    pub position: Option<f64>,

    /// Set the auto-archive threshold (seconds). Mutually exclusive with
    /// `--clear-auto-archive-after-seconds`.
    #[arg(
        long = "auto-archive-after-seconds",
        value_name = "SECONDS",
        conflicts_with = "clear_auto_archive_after_seconds"
    )]
    pub auto_archive_after_seconds: Option<i64>,

    /// Clear the auto-archive threshold.
    #[arg(long = "clear-auto-archive-after-seconds")]
    pub clear_auto_archive_after_seconds: bool,

    /// Set the per-status cap on simultaneously-active sessions.
    /// Mutually exclusive with `--clear-max-simultaneous-sessions`.
    #[arg(
        long = "max-simultaneous-sessions",
        value_name = "COUNT",
        conflicts_with = "clear_max_simultaneous_sessions"
    )]
    pub max_simultaneous_sessions: Option<u32>,

    /// Clear the per-status simultaneously-active sessions cap.
    #[arg(long = "clear-max-simultaneous-sessions")]
    pub clear_max_simultaneous_sessions: bool,

    /// Set the per-status container CPU limit (e.g. `500m`, `2`). Pass
    /// `--cpu-limit ""` to clear the override.
    #[arg(long = "cpu-limit", value_name = "STRING")]
    pub cpu_limit: Option<String>,

    /// Set the per-status container memory limit (e.g. `1Gi`, `512Mi`).
    /// Pass `--memory-limit ""` to clear the override.
    #[arg(long = "memory-limit", value_name = "STRING")]
    pub memory_limit: Option<String>,

    /// On-enter: set the issue's assignee to this principal. Accepts the
    /// canonical path form: `users/<name>`, `agents/<name>`, or
    /// `external/<system>/<name>`. If any `--on-enter-*` flag is
    /// present, the resulting `on_enter` is rebuilt from just those
    /// flags (other fields default).
    #[arg(
        long = "on-enter-assign-to",
        value_name = "PRINCIPAL",
        value_parser = parse_principal_arg,
        conflicts_with_all = ["on_enter_clear_assignee", "clear_on_enter"],
    )]
    pub on_enter_assign_to: Option<Principal>,

    /// On-enter: attach this form to the issue (doc-store path). See
    /// `--on-enter-assign-to` for the wholesale-rebuild semantics.
    #[arg(
        long = "on-enter-attach-form",
        value_name = "DOC_PATH",
        conflicts_with = "clear_on_enter"
    )]
    pub on_enter_attach_form: Option<DocumentPath>,

    /// On-enter: unset the issue's assignee. Mutually exclusive with
    /// `--on-enter-assign-to`.
    #[arg(long = "on-enter-clear-assignee", conflicts_with = "clear_on_enter")]
    pub on_enter_clear_assignee: bool,

    /// On-enter: tear down agent work attached to the issue — kill any
    /// `Created`/`Pending`/`Running` sessions and close any non-`Closed`
    /// conversations spawned from the issue.
    #[arg(long = "on-enter-teardown-work", conflicts_with = "clear_on_enter")]
    pub on_enter_teardown_work: bool,

    /// Clear the entire `on_enter` automation. Mutually exclusive with
    /// any `--on-enter-*` setter.
    #[arg(long = "clear-on-enter")]
    pub clear_on_enter: bool,
}

fn parse_principal_arg(value: &str) -> Result<Principal, String> {
    Principal::from_str(value).map_err(|err| {
        format!(
            "expected `users/<name>`, `agents/<name>`, or `external/<system>/<name>`; got '{value}' ({err})"
        )
    })
}

#[derive(Debug, Clone, Args)]
pub struct ArchiveStatusArgs {
    /// Project id (e.g. `j-abc123`) or key (e.g. `engineering`).
    #[arg(value_name = "PROJECT_ID_OR_KEY")]
    pub project_ref: ProjectRef,

    /// Status key (within the project) to archive / unarchive.
    #[arg(value_name = "STATUS_KEY")]
    pub status_key: StatusKey,
}

#[derive(Debug, Clone, Args)]
pub struct ArchiveProjectArgs {
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
        ProjectsCommand::Archive(args) => {
            let response = client
                .archive_project(&args.project_ref)
                .await
                .with_context(|| format!("failed to archive project '{}'", args.project_ref))?;
            write_archive_summary(context.output_format, &response, "Archived", &mut buffer)?;
        }
        ProjectsCommand::Unarchive(args) => {
            let response = client
                .unarchive_project(&args.project_ref)
                .await
                .with_context(|| format!("failed to unarchive project '{}'", args.project_ref))?;
            write_archive_summary(context.output_format, &response, "Unarchived", &mut buffer)?;
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
                let record = create_status(client, *args).await?;
                render(
                    ProjectRecords(&[record]),
                    context.output_format,
                    &mut buffer,
                )?;
            }
            StatusCommand::Update(args) => {
                let record = update_status(client, *args).await?;
                render(
                    ProjectRecords(&[record]),
                    context.output_format,
                    &mut buffer,
                )?;
            }
            StatusCommand::Archive(args) => {
                let record = archive_status(client, args).await?;
                render(
                    ProjectRecords(&[record]),
                    context.output_format,
                    &mut buffer,
                )?;
            }
            StatusCommand::Unarchive(args) => {
                let record = unarchive_status(client, args).await?;
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
    let creator = resolve_username(client).await?;
    let session_settings = build_create_session_settings(&args)?;
    let mut request = UpsertProjectRequest::new(args.key.clone(), args.name.clone());
    request.prompt_path = args.prompt_path.clone();
    request.session_settings = session_settings.clone();
    let response = client
        .create_project(&request)
        .await
        .context("failed to create project")?;
    let mut project = Project::new(args.key, args.name, Vec::new(), creator, false, 0.0);
    project.prompt_path = args.prompt_path;
    project.session_settings = session_settings;
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

    let prompt_path = apply_prompt_path_arg(
        args.prompt_path.clone(),
        current.project.prompt_path.clone(),
    );
    let session_settings =
        build_update_session_settings(&args, current.project.session_settings.clone())?;

    let mut request = UpsertProjectRequest::new(
        args.key
            .clone()
            .unwrap_or_else(|| current.project.key.clone()),
        args.name
            .clone()
            .unwrap_or_else(|| current.project.name.clone()),
    );
    request.prompt_path = prompt_path.clone();
    request.priority = current.project.priority;
    request.session_settings = session_settings.clone();

    let response = client
        .update_project(&args.project_ref, &request)
        .await
        .with_context(|| format!("failed to update project '{}'", args.project_ref))?;

    let mut project = Project::new(
        request.key,
        request.name,
        current.project.statuses,
        current.project.creator,
        current.project.archived,
        current.project.priority,
    );
    project.prompt_path = prompt_path;
    project.session_settings = session_settings;
    Ok(ProjectRecord::new(
        response.project_id,
        response.version,
        project,
    ))
}

/// Build the [`SessionSettings`] POSTed by `projects create` from the
/// flat per-field flags.
fn build_create_session_settings(args: &CreateProjectArgs) -> Result<SessionSettings> {
    let mut settings = SessionSettings::default();
    settings.cpu_limit = normalize_blank_string(args.cpu_limit.clone());
    settings.memory_limit = normalize_blank_string(args.memory_limit.clone());
    settings.image = normalize_blank_string(args.image.clone());
    settings.model = normalize_blank_string(args.model.clone());
    settings.max_retries = args.max_retries;
    if let Some(raw) = args.idle_timeout.as_deref() {
        settings.idle_timeout = Some(parse_idle_timeout(raw)?);
    }
    Ok(settings)
}

/// Build the [`SessionSettings`] PUT by `projects update`. Per-field
/// setters overlay the existing settings; `--clear-session-settings`
/// wipes everything; per-field clear flags (`--clear-max-retries`,
/// `--clear-idle-timeout`) drop a single field; passing an empty string
/// to a String-typed field clears that field.
fn build_update_session_settings(
    args: &UpdateProjectArgs,
    current: SessionSettings,
) -> Result<SessionSettings> {
    if args.clear_session_settings {
        return Ok(SessionSettings::default());
    }
    let mut settings = current;
    if let Some(value) = args.cpu_limit.clone() {
        settings.cpu_limit = if value.is_empty() { None } else { Some(value) };
    }
    if let Some(value) = args.memory_limit.clone() {
        settings.memory_limit = if value.is_empty() { None } else { Some(value) };
    }
    if let Some(value) = args.image.clone() {
        settings.image = if value.is_empty() { None } else { Some(value) };
    }
    if let Some(value) = args.model.clone() {
        settings.model = if value.is_empty() { None } else { Some(value) };
    }
    if args.clear_max_retries {
        settings.max_retries = None;
    } else if let Some(value) = args.max_retries {
        settings.max_retries = Some(value);
    }
    if args.clear_idle_timeout {
        settings.idle_timeout = None;
    } else if let Some(raw) = args.idle_timeout.as_deref() {
        settings.idle_timeout = Some(parse_idle_timeout(raw)?);
    }
    Ok(settings)
}

/// Parse an `--idle-timeout` value. Accepts `infinite` (case-insensitive)
/// for [`Timeout::Infinite`], or a positive whole number of seconds for
/// [`Timeout::Seconds`].
fn parse_idle_timeout(raw: &str) -> Result<Timeout> {
    if raw.eq_ignore_ascii_case("infinite") {
        return Ok(Timeout::Infinite);
    }
    let secs: u64 = raw.parse().with_context(|| {
        format!("expected `infinite` or a positive whole number of seconds; got '{raw}'")
    })?;
    Timeout::seconds(secs).ok_or_else(|| anyhow!("idle-timeout must be greater than zero seconds"))
}

async fn create_status(
    client: &dyn HydraClientInterface,
    args: CreateStatusArgs,
) -> Result<ProjectRecord> {
    let body = build_create_status_definition(&args)?;
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
    let body = build_update_status_definition(client, &args).await?;
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

/// Build the `StatusDefinition` POSTed by `projects status create` from
/// the direct `--key/--label/...` flags.
fn build_create_status_definition(args: &CreateStatusArgs) -> Result<StatusDefinition> {
    let on_enter = build_on_enter_from_flags(
        args.on_enter_assign_to.clone(),
        args.on_enter_attach_form.clone(),
        args.on_enter_clear_assignee,
        args.on_enter_teardown_work,
    )?;
    let mut def = StatusDefinition::new(
        args.key.clone(),
        args.label.clone(),
        args.color.clone(),
        args.unblocks_parents,
        args.unblocks_dependents,
        args.cascades_to_children,
        on_enter,
    );
    def.prompt_path = normalize_prompt_path(args.prompt_path.clone());
    def.interactive = args.interactive;
    def.suppress_sessions = args.suppress_sessions;
    def.auto_archive_after_seconds = args.auto_archive_after_seconds;
    def.max_simultaneous_sessions = args.max_simultaneous_sessions;
    def.position = args.position.unwrap_or(0.0);
    def.session_settings.cpu_limit = normalize_blank_string(args.cpu_limit.clone());
    def.session_settings.memory_limit = normalize_blank_string(args.memory_limit.clone());
    Ok(def)
}

/// Drop a `Some("")` flag value to `None` so empty CLI strings don't get
/// persisted as malformed overrides. Mirrors [`normalize_prompt_path`].
fn normalize_blank_string(arg: Option<String>) -> Option<String> {
    match arg {
        None => None,
        Some(value) if value.is_empty() => None,
        Some(value) => Some(value),
    }
}

/// Build the `StatusDefinition` PUT by `projects status update`. The
/// existing definition is fetched from the server and the user's flags
/// are overlaid on top. Errors when no direct flag is set so the caller
/// gets a clear message instead of a no-op update.
async fn build_update_status_definition(
    client: &dyn HydraClientInterface,
    args: &UpdateStatusArgs,
) -> Result<StatusDefinition> {
    if !update_has_any_direct_flag(args) {
        bail!(
            "no updates specified; use --key, --label, --color, --prompt-path, --position, \
             --auto-archive-after-seconds, --clear-auto-archive-after-seconds, \
             --max-simultaneous-sessions, --clear-max-simultaneous-sessions, \
             --unblocks-parents=<bool>, --unblocks-dependents=<bool>, \
             --cascades-to-children=<bool>, --interactive=<bool>, \
             --suppress-sessions=<bool>, --cpu-limit, --memory-limit, \
             --on-enter-*, or --clear-on-enter"
        );
    }
    let current_project = client
        .get_project(&args.project_ref)
        .await
        .with_context(|| {
            format!(
                "failed to fetch project '{}' to overlay status update",
                args.project_ref
            )
        })?;
    let current = current_project
        .project
        .statuses
        .iter()
        .find(|s| s.key == args.status_key)
        .ok_or_else(|| {
            anyhow!(
                "status '{}' not found on project '{}'",
                args.status_key,
                args.project_ref
            )
        })?
        .clone();
    apply_update_overlay(args, current)
}

fn update_has_any_direct_flag(args: &UpdateStatusArgs) -> bool {
    args.key.is_some()
        || args.label.is_some()
        || args.color.is_some()
        || args.prompt_path.is_some()
        || args.unblocks_parents.is_some()
        || args.unblocks_dependents.is_some()
        || args.cascades_to_children.is_some()
        || args.interactive.is_some()
        || args.suppress_sessions.is_some()
        || args.position.is_some()
        || args.auto_archive_after_seconds.is_some()
        || args.clear_auto_archive_after_seconds
        || args.max_simultaneous_sessions.is_some()
        || args.clear_max_simultaneous_sessions
        || args.cpu_limit.is_some()
        || args.memory_limit.is_some()
        || args.on_enter_assign_to.is_some()
        || args.on_enter_attach_form.is_some()
        || args.on_enter_clear_assignee
        || args.on_enter_teardown_work
        || args.clear_on_enter
}

/// Overlay the user's direct flags on top of an existing
/// `StatusDefinition`. Fields the user did not name are preserved
/// verbatim. The `on_enter` group is rebuilt wholesale when any
/// `--on-enter-*` setter is present (see [`overlay_on_enter`]).
fn apply_update_overlay(
    args: &UpdateStatusArgs,
    current: StatusDefinition,
) -> Result<StatusDefinition> {
    let mut def = current;
    if let Some(k) = args.key.clone() {
        def.key = k;
    }
    if let Some(l) = args.label.clone() {
        def.label = l;
    }
    if let Some(c) = args.color.clone() {
        def.color = c;
    }
    def.prompt_path = apply_prompt_path_arg(args.prompt_path.clone(), def.prompt_path.clone());
    if let Some(v) = args.unblocks_parents {
        def.unblocks_parents = v;
    }
    if let Some(v) = args.unblocks_dependents {
        def.unblocks_dependents = v;
    }
    if let Some(v) = args.cascades_to_children {
        def.cascades_to_children = v;
    }
    if let Some(v) = args.interactive {
        def.interactive = v;
    }
    if let Some(v) = args.suppress_sessions {
        def.suppress_sessions = v;
    }
    if let Some(v) = args.position {
        def.position = v;
    }
    if args.clear_auto_archive_after_seconds {
        def.auto_archive_after_seconds = None;
    } else if let Some(v) = args.auto_archive_after_seconds {
        def.auto_archive_after_seconds = Some(v);
    }
    if args.clear_max_simultaneous_sessions {
        def.max_simultaneous_sessions = None;
    } else if let Some(v) = args.max_simultaneous_sessions {
        def.max_simultaneous_sessions = Some(v);
    }
    if let Some(value) = args.cpu_limit.clone() {
        def.session_settings.cpu_limit = if value.is_empty() { None } else { Some(value) };
    }
    if let Some(value) = args.memory_limit.clone() {
        def.session_settings.memory_limit = if value.is_empty() { None } else { Some(value) };
    }
    def.on_enter = overlay_on_enter(args, def.on_enter)?;
    Ok(def)
}

/// Build a [`StatusOnEnter`] from the four `--on-enter-*` flag values.
/// Returns `None` when no flag is set; otherwise returns the constructed
/// automation. Rejects configurations that fail [`StatusOnEnter::validate`].
fn build_on_enter_from_flags(
    assign_to: Option<Principal>,
    attach_form: Option<DocumentPath>,
    clear_assignee: bool,
    teardown_work: bool,
) -> Result<Option<StatusOnEnter>> {
    if assign_to.is_none() && attach_form.is_none() && !clear_assignee && !teardown_work {
        return Ok(None);
    }
    let mut on_enter = StatusOnEnter::new(assign_to, attach_form);
    on_enter.clear_assignee = clear_assignee;
    on_enter.teardown_work = teardown_work;
    on_enter
        .validate()
        .map_err(|e| anyhow!("invalid on_enter configuration: {e}"))?;
    Ok(Some(on_enter))
}

/// Compute the `on_enter` field for an update. `--clear-on-enter` wipes
/// the automation; otherwise, if any `--on-enter-*` setter is present
/// the result is rebuilt wholesale; otherwise the existing value is
/// preserved.
fn overlay_on_enter(
    args: &UpdateStatusArgs,
    current: Option<StatusOnEnter>,
) -> Result<Option<StatusOnEnter>> {
    if args.clear_on_enter {
        return Ok(None);
    }
    let any_setter = args.on_enter_assign_to.is_some()
        || args.on_enter_attach_form.is_some()
        || args.on_enter_clear_assignee
        || args.on_enter_teardown_work;
    if !any_setter {
        return Ok(current);
    }
    build_on_enter_from_flags(
        args.on_enter_assign_to.clone(),
        args.on_enter_attach_form.clone(),
        args.on_enter_clear_assignee,
        args.on_enter_teardown_work,
    )
}

/// Treat a `--prompt-path ""` on create as "no value". Used to mirror
/// the [`apply_prompt_path_arg`] update semantics in create mode (a
/// stored `Some("")` would just be a malformed path anyway).
fn normalize_prompt_path(arg: Option<String>) -> Option<String> {
    match arg {
        None => None,
        Some(value) if value.is_empty() => None,
        Some(value) => Some(value),
    }
}

async fn archive_status(
    client: &dyn HydraClientInterface,
    args: ArchiveStatusArgs,
) -> Result<ProjectRecord> {
    client
        .archive_project_status(&args.project_ref, &args.status_key)
        .await
        .with_context(|| {
            format!(
                "failed to archive status '{}' on project '{}'",
                args.status_key, args.project_ref
            )
        })?;
    let record = client
        .get_project(&args.project_ref)
        .await
        .with_context(|| format!("failed to fetch project '{}'", args.project_ref))?;
    Ok(record)
}

async fn unarchive_status(
    client: &dyn HydraClientInterface,
    args: ArchiveStatusArgs,
) -> Result<ProjectRecord> {
    client
        .unarchive_project_status(&args.project_ref, &args.status_key)
        .await
        .with_context(|| {
            format!(
                "failed to unarchive status '{}' on project '{}'",
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

fn write_archive_summary<W: std::io::Write>(
    format: ResolvedOutputFormat,
    response: &UpsertProjectResponse,
    verb: &str,
    writer: &mut W,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Pretty => {
            writeln!(
                writer,
                "{verb} project '{}' (version {})",
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

    // --- direct-flag plumbing ----------------------------------------

    use clap::{CommandFactory, FromArgMatches, Parser};

    fn status_key(s: &str) -> StatusKey {
        StatusKey::try_new(s).unwrap()
    }

    #[derive(Debug, Parser)]
    struct CreateHarness {
        #[command(flatten)]
        args: CreateStatusArgs,
    }

    #[derive(Debug, Parser)]
    struct UpdateHarness {
        #[command(flatten)]
        args: UpdateStatusArgs,
    }

    fn parse_create(argv: &[&str]) -> Result<CreateStatusArgs, clap::Error> {
        let mut full = vec!["status-create", "engineering"];
        full.extend(argv);
        CreateHarness::try_parse_from(full).map(|h| h.args)
    }

    fn parse_create_failure(argv: &[&str]) -> clap::Error {
        let mut full = vec!["status-create", "engineering"];
        full.extend(argv);
        // Use try_get_matches_from on the underlying clap Command so we
        // get the original error kind instead of an exit-stub.
        let command = CreateHarness::command();
        let matches = command.try_get_matches_from(full);
        match matches {
            Err(e) => e,
            Ok(m) => match CreateHarness::from_arg_matches(&m) {
                Err(e) => e,
                Ok(_) => panic!("expected clap error, parse succeeded"),
            },
        }
    }

    fn parse_update(argv: &[&str]) -> Result<UpdateStatusArgs, clap::Error> {
        let mut full = vec!["status-update", "engineering", "backlog"];
        full.extend(argv);
        UpdateHarness::try_parse_from(full).map(|h| h.args)
    }

    fn parse_update_failure(argv: &[&str]) -> clap::Error {
        let mut full = vec!["status-update", "engineering", "backlog"];
        full.extend(argv);
        let command = UpdateHarness::command();
        let matches = command.try_get_matches_from(full);
        match matches {
            Err(e) => e,
            Ok(m) => match UpdateHarness::from_arg_matches(&m) {
                Err(e) => e,
                Ok(_) => panic!("expected clap error, parse succeeded"),
            },
        }
    }

    fn current_status_with_on_enter() -> StatusDefinition {
        let mut def = StatusDefinition::new(
            status_key("backlog"),
            "Backlog".into(),
            "#9b59b6".parse().unwrap(),
            false,
            false,
            true,
            Some(StatusOnEnter::new(
                Some(Principal::Agent {
                    name: hydra_common::api::v1::agents::AgentName::try_new("pm").unwrap(),
                }),
                None,
            )),
        );
        def.prompt_path = Some("/projects/eng/statuses/backlog.md".into());
        def.interactive = true;
        def.auto_archive_after_seconds = Some(3600);
        def.position = 2.0;
        def
    }

    // --- create-mode tests -------------------------------------------

    #[test]
    fn create_required_flags() {
        let err = parse_create_failure(&[]);
        let msg = err.to_string();
        assert!(
            msg.contains("--key") || msg.contains("required"),
            "expected required-flag error, got: {msg}",
        );
    }

    #[test]
    fn create_on_enter_clear_assignee_conflicts_with_assign_to() {
        let err = parse_create_failure(&[
            "--key",
            "backlog",
            "--label",
            "Backlog",
            "--color",
            "#aabbcc",
            "--on-enter-assign-to",
            "agents/swe",
            "--on-enter-clear-assignee",
        ]);
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn build_create_status_definition_from_direct_flags() {
        let args = parse_create(&[
            "--key",
            "inbox",
            "--label",
            "Inbox",
            "--color",
            "#aabbcc",
            "--position",
            "1.5",
            "--auto-archive-after-seconds",
            "120",
            "--prompt-path",
            "/projects/eng/statuses/inbox.md",
            "--interactive",
            "--suppress-sessions",
            "--unblocks-parents",
        ])
        .unwrap();
        let def = build_create_status_definition(&args).unwrap();
        assert_eq!(def.key, status_key("inbox"));
        assert_eq!(def.label, "Inbox");
        assert_eq!(def.color.as_ref(), "#aabbcc");
        assert!(def.unblocks_parents);
        assert!(!def.unblocks_dependents);
        assert!(!def.cascades_to_children);
        assert!(def.interactive);
        assert!(def.suppress_sessions);
        assert_eq!(def.position, 1.5);
        assert_eq!(def.auto_archive_after_seconds, Some(120));
        assert_eq!(
            def.prompt_path.as_deref(),
            Some("/projects/eng/statuses/inbox.md"),
        );
        assert!(def.on_enter.is_none());
    }

    #[test]
    fn build_create_status_definition_default_position_is_zero() {
        let args =
            parse_create(&["--key", "inbox", "--label", "Inbox", "--color", "#aabbcc"]).unwrap();
        let def = build_create_status_definition(&args).unwrap();
        assert_eq!(def.position, 0.0);
        assert!(def.on_enter.is_none());
        assert_eq!(def.auto_archive_after_seconds, None);
        assert!(!def.suppress_sessions);
    }

    #[test]
    fn build_create_status_definition_on_enter_with_agent_assign_to() {
        let args = parse_create(&[
            "--key",
            "review",
            "--label",
            "Review",
            "--color",
            "#aabbcc",
            "--on-enter-assign-to",
            "agents/swe",
            "--on-enter-teardown-work",
        ])
        .unwrap();
        let def = build_create_status_definition(&args).unwrap();
        let on_enter = def.on_enter.expect("on_enter present");
        match on_enter.assign_to.expect("assign_to present") {
            Principal::Agent { name } => assert_eq!(name.as_str(), "swe"),
            other => panic!("expected agent principal, got {other:?}"),
        }
        assert!(on_enter.teardown_work);
        assert!(!on_enter.clear_assignee);
    }

    #[test]
    fn build_create_status_definition_on_enter_clear_assignee_only() {
        let args = parse_create(&[
            "--key",
            "review",
            "--label",
            "Review",
            "--color",
            "#aabbcc",
            "--on-enter-clear-assignee",
        ])
        .unwrap();
        let def = build_create_status_definition(&args).unwrap();
        let on_enter = def.on_enter.expect("on_enter present");
        assert!(on_enter.clear_assignee);
        assert!(on_enter.assign_to.is_none());
    }

    #[test]
    fn create_invalid_principal_is_rejected_at_parse_time() {
        let err = parse_create_failure(&[
            "--key",
            "review",
            "--label",
            "Review",
            "--color",
            "#aabbcc",
            "--on-enter-assign-to",
            "alice",
        ]);
        let msg = err.to_string();
        assert!(
            msg.contains("agents/<name>") || msg.contains("users/<name>"),
            "expected hint about path form, got: {msg}",
        );
    }

    // --- update-mode tests -------------------------------------------

    #[test]
    fn update_no_direct_flag_rejected_with_clear_error() {
        let args = parse_update(&[]).expect("clap accepts zero flags");
        // The 'no updates specified' error is raised by the builder,
        // not clap, because clap can't express required-at-least-one
        // semantics without forcing the user to learn the field
        // list. The error mirrors `documents update`.
        let err = build_update_status_definition_sync(&args).unwrap_err();
        assert!(
            err.to_string().contains("no updates specified"),
            "got: {err}",
        );
    }

    #[test]
    fn update_on_enter_clear_assignee_conflicts_with_assign_to() {
        let err = parse_update_failure(&[
            "--on-enter-assign-to",
            "agents/swe",
            "--on-enter-clear-assignee",
        ]);
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn update_clear_on_enter_conflicts_with_any_on_enter_setter() {
        let cases: &[&[&str]] = &[
            &["--clear-on-enter", "--on-enter-assign-to", "agents/swe"],
            &["--clear-on-enter", "--on-enter-attach-form", "/forms/x.md"],
            &["--clear-on-enter", "--on-enter-clear-assignee"],
            &["--clear-on-enter", "--on-enter-teardown-work"],
        ];
        for argv in cases {
            let err = parse_update_failure(argv);
            assert_eq!(
                err.kind(),
                clap::error::ErrorKind::ArgumentConflict,
                "argv: {argv:?}",
            );
        }
    }

    #[test]
    fn update_clear_auto_archive_conflicts_with_setter() {
        let err = parse_update_failure(&[
            "--auto-archive-after-seconds",
            "60",
            "--clear-auto-archive-after-seconds",
        ]);
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn update_bool_flag_requires_explicit_value() {
        let err = parse_update_failure(&["--unblocks-parents"]);
        // Either a value-missing or unknown-arg error — both signal
        // that bare `--unblocks-parents` is not accepted.
        let kind = err.kind();
        assert!(
            matches!(
                kind,
                clap::error::ErrorKind::InvalidValue
                    | clap::error::ErrorKind::MissingRequiredArgument
                    | clap::error::ErrorKind::ValueValidation
                    | clap::error::ErrorKind::WrongNumberOfValues
                    | clap::error::ErrorKind::UnknownArgument
            ),
            "expected value-required error, got: {kind:?}",
        );
    }

    #[test]
    fn update_bool_flag_with_explicit_value_parses() {
        let args = parse_update(&["--unblocks-parents=true"]).unwrap();
        assert_eq!(args.unblocks_parents, Some(true));
        let args = parse_update(&["--unblocks-parents=false"]).unwrap();
        assert_eq!(args.unblocks_parents, Some(false));
    }

    #[test]
    fn update_suppress_sessions_parses_explicit_value() {
        let args = parse_update(&["--suppress-sessions=true"]).unwrap();
        assert_eq!(args.suppress_sessions, Some(true));
        let args = parse_update(&["--suppress-sessions=false"]).unwrap();
        assert_eq!(args.suppress_sessions, Some(false));
    }

    #[test]
    fn update_suppress_sessions_bare_form_rejected() {
        let err = parse_update_failure(&["--suppress-sessions"]);
        let kind = err.kind();
        assert!(
            matches!(
                kind,
                clap::error::ErrorKind::InvalidValue
                    | clap::error::ErrorKind::MissingRequiredArgument
                    | clap::error::ErrorKind::ValueValidation
                    | clap::error::ErrorKind::WrongNumberOfValues
                    | clap::error::ErrorKind::UnknownArgument
            ),
            "expected value-required error, got: {kind:?}",
        );
    }

    #[test]
    fn update_suppress_sessions_alone_does_not_trigger_no_updates_bail() {
        let args = parse_update(&["--suppress-sessions=true"]).unwrap();
        assert!(update_has_any_direct_flag(&args));
    }

    /// Sync mirror of `build_update_status_definition` for use in unit
    /// tests: skips the server fetch, exercising only the overlay /
    /// validation paths.
    fn build_update_status_definition_sync(args: &UpdateStatusArgs) -> Result<StatusDefinition> {
        if !update_has_any_direct_flag(args) {
            bail!("no updates specified; use --key, --label, --color, --prompt-path, --position");
        }
        apply_update_overlay(args, current_status_with_on_enter())
    }

    #[test]
    fn update_overlay_label_preserves_everything_else() {
        let args = parse_update(&["--label", "Refreshed Backlog"]).unwrap();
        let def = build_update_status_definition_sync(&args).unwrap();
        let base = current_status_with_on_enter();
        assert_eq!(def.label, "Refreshed Backlog");
        assert_eq!(def.key, base.key);
        assert_eq!(def.color, base.color);
        assert_eq!(def.prompt_path, base.prompt_path);
        assert_eq!(def.interactive, base.interactive);
        assert_eq!(def.cascades_to_children, base.cascades_to_children);
        assert_eq!(def.position, base.position);
        assert_eq!(
            def.auto_archive_after_seconds,
            base.auto_archive_after_seconds
        );
        assert_eq!(def.on_enter, base.on_enter);
    }

    #[test]
    fn update_overlay_rename_via_key_flag() {
        let args = parse_update(&["--key", "in-review"]).unwrap();
        let def = build_update_status_definition_sync(&args).unwrap();
        assert_eq!(def.key, status_key("in-review"));
    }

    #[test]
    fn update_overlay_clear_prompt_path_via_empty_string() {
        let args = parse_update(&["--prompt-path", ""]).unwrap();
        let def = build_update_status_definition_sync(&args).unwrap();
        assert!(def.prompt_path.is_none());
    }

    #[test]
    fn update_overlay_clear_auto_archive_flag() {
        let args = parse_update(&["--clear-auto-archive-after-seconds"]).unwrap();
        let def = build_update_status_definition_sync(&args).unwrap();
        assert!(def.auto_archive_after_seconds.is_none());
    }

    #[test]
    fn create_max_simultaneous_sessions_flag_parses() {
        let args = parse_create(&[
            "--key",
            "inbox",
            "--label",
            "Inbox",
            "--color",
            "#aabbcc",
            "--max-simultaneous-sessions",
            "5",
        ])
        .unwrap();
        let def = build_create_status_definition(&args).unwrap();
        assert_eq!(def.max_simultaneous_sessions, Some(5));
    }

    #[test]
    fn create_max_simultaneous_sessions_defaults_to_none() {
        let args =
            parse_create(&["--key", "inbox", "--label", "Inbox", "--color", "#aabbcc"]).unwrap();
        let def = build_create_status_definition(&args).unwrap();
        assert_eq!(def.max_simultaneous_sessions, None);
    }

    #[test]
    fn update_overlay_set_max_simultaneous_sessions() {
        let args = parse_update(&["--max-simultaneous-sessions", "7"]).unwrap();
        let def = build_update_status_definition_sync(&args).unwrap();
        assert_eq!(def.max_simultaneous_sessions, Some(7));
    }

    #[test]
    fn update_overlay_clear_max_simultaneous_sessions() {
        let mut base = current_status_with_on_enter();
        base.max_simultaneous_sessions = Some(3);
        let args = parse_update(&["--clear-max-simultaneous-sessions"]).unwrap();
        let def = apply_update_overlay(&args, base).unwrap();
        assert!(def.max_simultaneous_sessions.is_none());
    }

    #[test]
    fn update_clear_max_simultaneous_sessions_conflicts_with_setter() {
        let err = parse_update_failure(&[
            "--max-simultaneous-sessions",
            "5",
            "--clear-max-simultaneous-sessions",
        ]);
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn update_overlay_max_simultaneous_sessions_preserved_when_flag_absent() {
        let mut base = current_status_with_on_enter();
        base.max_simultaneous_sessions = Some(4);
        let args = parse_update(&["--label", "Refreshed"]).unwrap();
        let def = apply_update_overlay(&args, base).unwrap();
        assert_eq!(def.max_simultaneous_sessions, Some(4));
    }

    #[test]
    fn update_overlay_set_auto_archive_flag() {
        let args = parse_update(&["--auto-archive-after-seconds", "9999"]).unwrap();
        let def = build_update_status_definition_sync(&args).unwrap();
        assert_eq!(def.auto_archive_after_seconds, Some(9999));
    }

    #[test]
    fn update_overlay_bool_flags_toggle() {
        let args =
            parse_update(&["--unblocks-parents=true", "--cascades-to-children=false"]).unwrap();
        let def = build_update_status_definition_sync(&args).unwrap();
        assert!(def.unblocks_parents);
        assert!(!def.cascades_to_children);
    }

    #[test]
    fn update_overlay_suppress_sessions_flips_existing_value() {
        let mut base = current_status_with_on_enter();
        base.suppress_sessions = true;
        let args = parse_update(&["--suppress-sessions=false"]).unwrap();
        let def = apply_update_overlay(&args, base).unwrap();
        assert!(!def.suppress_sessions);
    }

    #[test]
    fn update_overlay_suppress_sessions_preserved_when_flag_absent() {
        let mut base = current_status_with_on_enter();
        base.suppress_sessions = true;
        let args = parse_update(&["--label", "Refreshed"]).unwrap();
        let def = apply_update_overlay(&args, base).unwrap();
        assert!(def.suppress_sessions);
    }

    #[test]
    fn update_overlay_on_enter_rebuilt_wholesale() {
        // Current on_enter has assign_to=agents/pm. After --on-enter-teardown-work
        // (and nothing else), the on_enter is rebuilt from scratch so
        // assign_to is None and teardown_work=true.
        let args = parse_update(&["--on-enter-teardown-work"]).unwrap();
        let def = build_update_status_definition_sync(&args).unwrap();
        let on_enter = def.on_enter.expect("on_enter present");
        assert!(on_enter.assign_to.is_none());
        assert!(on_enter.teardown_work);
        assert!(!on_enter.clear_assignee);
    }

    #[test]
    fn update_overlay_clear_on_enter() {
        let args = parse_update(&["--clear-on-enter"]).unwrap();
        let def = build_update_status_definition_sync(&args).unwrap();
        assert!(def.on_enter.is_none());
    }

    #[test]
    fn update_overlay_on_enter_with_assign_to_agent() {
        let args = parse_update(&["--on-enter-assign-to", "agents/swe"]).unwrap();
        let def = build_update_status_definition_sync(&args).unwrap();
        let on_enter = def.on_enter.expect("on_enter present");
        match on_enter.assign_to.expect("assign_to present") {
            Principal::Agent { name } => assert_eq!(name.as_str(), "swe"),
            other => panic!("expected agent principal, got {other:?}"),
        }
    }

    #[test]
    fn update_overlay_on_enter_preserved_when_no_on_enter_flag() {
        let args = parse_update(&["--label", "Refreshed"]).unwrap();
        let def = build_update_status_definition_sync(&args).unwrap();
        assert_eq!(def.on_enter, current_status_with_on_enter().on_enter);
    }

    // --- project-level session_settings CLI flag tests ----------------

    #[derive(Debug, Parser)]
    struct CreateProjectHarness {
        #[command(flatten)]
        args: CreateProjectArgs,
    }

    #[derive(Debug, Parser)]
    struct UpdateProjectHarness {
        #[command(flatten)]
        args: UpdateProjectArgs,
    }

    fn parse_create_project(argv: &[&str]) -> Result<CreateProjectArgs, clap::Error> {
        let mut full = vec![
            "create-project",
            "--key",
            "engineering",
            "--name",
            "Engineering",
        ];
        full.extend(argv);
        CreateProjectHarness::try_parse_from(full).map(|h| h.args)
    }

    fn parse_update_project(argv: &[&str]) -> Result<UpdateProjectArgs, clap::Error> {
        let mut full = vec!["update-project", "engineering"];
        full.extend(argv);
        UpdateProjectHarness::try_parse_from(full).map(|h| h.args)
    }

    fn parse_update_project_failure(argv: &[&str]) -> clap::Error {
        let mut full = vec!["update-project", "engineering"];
        full.extend(argv);
        let command = UpdateProjectHarness::command();
        let matches = command.try_get_matches_from(full);
        match matches {
            Err(e) => e,
            Ok(m) => match UpdateProjectHarness::from_arg_matches(&m) {
                Err(e) => e,
                Ok(_) => panic!("expected clap error, parse succeeded"),
            },
        }
    }

    #[test]
    fn create_project_flat_flags_populate_session_settings() {
        let args = parse_create_project(&[
            "--cpu-limit",
            "500m",
            "--memory-limit",
            "256Mi",
            "--image",
            "hydra-worker:project",
            "--model",
            "anthropic/claude",
            "--max-retries",
            "4",
            "--idle-timeout",
            "infinite",
        ])
        .unwrap();
        let settings = build_create_session_settings(&args).unwrap();
        assert_eq!(settings.cpu_limit.as_deref(), Some("500m"));
        assert_eq!(settings.memory_limit.as_deref(), Some("256Mi"));
        assert_eq!(settings.image.as_deref(), Some("hydra-worker:project"));
        assert_eq!(settings.model.as_deref(), Some("anthropic/claude"));
        assert_eq!(settings.max_retries, Some(4));
        assert_eq!(settings.idle_timeout, Some(Timeout::Infinite));
    }

    #[test]
    fn create_project_idle_timeout_seconds_parses() {
        let args = parse_create_project(&["--idle-timeout", "1800"]).unwrap();
        let settings = build_create_session_settings(&args).unwrap();
        assert_eq!(settings.idle_timeout, Some(Timeout::seconds(1800).unwrap()));
    }

    #[test]
    fn create_project_idle_timeout_rejects_zero() {
        let args = parse_create_project(&["--idle-timeout", "0"]).unwrap();
        let err = build_create_session_settings(&args).unwrap_err();
        assert!(err.to_string().contains("greater than zero"));
    }

    #[test]
    fn create_project_idle_timeout_rejects_garbage() {
        let args = parse_create_project(&["--idle-timeout", "soon"]).unwrap();
        let err = build_create_session_settings(&args).unwrap_err();
        assert!(err.to_string().contains("expected `infinite`"));
    }

    #[test]
    fn create_project_blank_string_drops_session_settings_field() {
        let args = parse_create_project(&["--cpu-limit", "", "--image", ""]).unwrap();
        let settings = build_create_session_settings(&args).unwrap();
        assert!(settings.cpu_limit.is_none());
        assert!(settings.image.is_none());
    }

    #[test]
    fn update_project_flat_flags_overlay_existing_session_settings() {
        let mut current = SessionSettings::default();
        current.cpu_limit = Some("100m".into());
        current.memory_limit = Some("128Mi".into());
        current.image = Some("old-image".into());
        let args = parse_update_project(&["--image", "new-image", "--cpu-limit", "500m"]).unwrap();
        let settings = build_update_session_settings(&args, current).unwrap();
        // Updated fields use new values.
        assert_eq!(settings.image.as_deref(), Some("new-image"));
        assert_eq!(settings.cpu_limit.as_deref(), Some("500m"));
        // Untouched field is preserved.
        assert_eq!(settings.memory_limit.as_deref(), Some("128Mi"));
    }

    #[test]
    fn update_project_blank_string_clears_session_settings_field() {
        let mut current = SessionSettings::default();
        current.cpu_limit = Some("100m".into());
        let args = parse_update_project(&["--cpu-limit", ""]).unwrap();
        let settings = build_update_session_settings(&args, current).unwrap();
        assert!(settings.cpu_limit.is_none());
    }

    #[test]
    fn update_project_clear_max_retries_drops_only_max_retries() {
        let mut current = SessionSettings::default();
        current.max_retries = Some(5);
        current.cpu_limit = Some("100m".into());
        let args = parse_update_project(&["--clear-max-retries"]).unwrap();
        let settings = build_update_session_settings(&args, current).unwrap();
        assert!(settings.max_retries.is_none());
        assert_eq!(settings.cpu_limit.as_deref(), Some("100m"));
    }

    #[test]
    fn update_project_clear_idle_timeout_drops_only_idle_timeout() {
        let mut current = SessionSettings::default();
        current.idle_timeout = Some(Timeout::Infinite);
        current.cpu_limit = Some("100m".into());
        let args = parse_update_project(&["--clear-idle-timeout"]).unwrap();
        let settings = build_update_session_settings(&args, current).unwrap();
        assert!(settings.idle_timeout.is_none());
        assert_eq!(settings.cpu_limit.as_deref(), Some("100m"));
    }

    #[test]
    fn update_project_clear_session_settings_wipes_everything() {
        let mut current = SessionSettings::default();
        current.image = Some("old-image".into());
        current.cpu_limit = Some("100m".into());
        current.max_retries = Some(5);
        current.idle_timeout = Some(Timeout::Infinite);
        let args = parse_update_project(&["--clear-session-settings"]).unwrap();
        let settings = build_update_session_settings(&args, current).unwrap();
        assert!(SessionSettings::is_default(&settings));
    }

    #[test]
    fn update_project_clear_session_settings_conflicts_with_per_field_flags() {
        let err =
            parse_update_project_failure(&["--clear-session-settings", "--cpu-limit", "500m"]);
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn update_project_clear_max_retries_conflicts_with_max_retries() {
        let err = parse_update_project_failure(&["--clear-max-retries", "--max-retries", "5"]);
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn update_project_clear_idle_timeout_conflicts_with_idle_timeout() {
        let err =
            parse_update_project_failure(&["--clear-idle-timeout", "--idle-timeout", "infinite"]);
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }
}
