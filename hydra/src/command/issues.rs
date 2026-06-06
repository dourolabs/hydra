use crate::{
    client::HydraClientInterface,
    command::{
        output::{
            render, CommandContext, DeletedIssueOutcome, IssueRecords, IssueSummaryRecords,
            ResolvedOutputFormat, SubmitFormOutcome,
        },
        projects::ProjectRef,
        utils::resolve_username,
    },
    output_writer::write_stdout,
};
use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use clap::Subcommand;
use hydra_common::{
    api::v1::labels::{Label, SearchLabelsQuery, UpsertLabelRequest},
    api::v1::projects::StatusKey,
    constants::ENV_HYDRA_ISSUE_ID,
    form::Form,
    issues::{
        Issue, IssueDependency, IssueDependencyType, IssueId, IssueSummaryRecord, IssueType,
        IssueVersionRecord, SearchIssuesQuery, SessionSettings, SubmitFormRequest,
        UpsertIssueRequest,
    },
    principal::{Principal, PrincipalParseError},
    users::Username,
    HydraId, LabelId, PatchId, ProjectId, RelativeVersionNumber, RepoName,
};
use std::str::FromStr;

/// Tri-state for the `--project` / `--clear-project` pair on `hydra issues update`:
/// `Unchanged` keeps the existing value, `Set` replaces it, `Clear` detaches.
enum ProjectUpdate {
    Unchanged,
    Set(ProjectId),
    Clear,
}

/// clap value parser for `--assignee`. Phase 4b requires the full
/// canonical path form (`users/<name>`, `agents/<name>`, or
/// `external/<system>/<name>`). Bare strings (e.g. `alice`) are rejected
/// with a hint pointing at the right form.
fn parse_assignee_principal(value: &str) -> Result<Principal, String> {
    Principal::from_str(value).map_err(|err| {
        let hint = match err {
            PrincipalParseError::UnknownKind(_) if !value.contains('/') => {
                format!(" (got: '{value}'; did you mean 'users/{value}'?)")
            }
            _ => String::new(),
        };
        format!(
            "--assignee requires a full path: users/<name>, agents/<name>, or external/<system>/<name>{hint}"
        )
    })
}

// The `Update` variant has many optional flags by design (one per field of
// `Issue` plus matching `clear-*` toggles). Boxing each variant individually
// would clutter dispatch; the CLI parses one of these per invocation, so the
// stack cost is irrelevant.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Subcommand)]
pub enum IssueCommands {
    /// List Hydra issues. Returns summary records with truncated descriptions; use `get` for complete details.
    List {
        /// Filter by issue ID.
        #[arg(long, value_name = "ISSUE_ID", conflicts_with = "query")]
        id: Option<IssueId>,

        /// Filter by issue type.
        #[arg(long, value_name = "ISSUE_TYPE")]
        r#type: Option<IssueType>,

        /// Filter by issue status key (comma-separated). Accepts any
        /// status key declared by an issue's project — `open`,
        /// `in-progress`, `closed`, `dropped`, `failed` for the default
        /// project, plus bespoke per-project keys (e.g. `inbox`, `triage`).
        #[arg(long, value_name = "STATUS_KEY", value_delimiter = ',')]
        status: Vec<StatusKey>,

        /// Scope results to a single project by id or key (e.g.
        /// `j-engineering` or `engineering-v2`).
        #[arg(long, value_name = "PROJECT_ID_OR_KEY")]
        project: Option<ProjectRef>,

        /// Filter by assignee. Requires the full canonical path form:
        /// `users/<name>`, `agents/<name>`, or `external/<system>/<name>`.
        #[arg(long, value_name = "ASSIGNEE", value_parser = parse_assignee_principal)]
        assignee: Option<Principal>,

        /// Search by query string.
        #[arg(long, value_name = "QUERY")]
        query: Option<String>,

        /// Filter by label names (comma-separated).
        #[arg(long = "labels", value_name = "LABEL_NAME", value_delimiter = ',')]
        labels: Vec<String>,

        /// Include deleted issues in the listing.
        #[arg(long = "include-deleted")]
        include_deleted: bool,
    },
    /// Create a new issue.
    Create {
        /// Issue type: bug, feature, task, chore, merge-request, or review-request (defaults to task).
        #[arg(long, value_name = "ISSUE_TYPE", default_value_t = IssueType::Task)]
        r#type: IssueType,

        /// Short title for the issue.
        #[arg(long, value_name = "TITLE")]
        title: Option<String>,

        /// Issue status key. Validity depends on the issue's project:
        /// default-project issues accept `open`, `in-progress`, `closed`,
        /// `dropped`, `failed`; bespoke projects accept any of their declared
        /// status keys. The server validates the value. Defaults to `open`.
        #[arg(long, value_name = "STATUS_KEY", default_value = "open")]
        status: StatusKey,

        /// Project id or key to attach this issue to. Omit for the
        /// synthesized default project.
        #[arg(long, value_name = "PROJECT_ID_OR_KEY")]
        project: Option<ProjectRef>,

        /// Issue dependencies in the format dependency-type:ISSUE_ID where dependency-type is child-of or blocked-on (e.g. child-of:i-abcd).
        #[arg(long = "deps", value_name = "TYPE:ISSUE_ID", value_parser = parse_issue_dependency)]
        dependencies: Vec<IssueDependency>,

        /// Patch ids to associate with the issue.
        #[arg(long = "patches", value_name = "PATCH_ID", value_delimiter = ',')]
        patches: Vec<PatchId>,

        /// Assignee for the issue. Requires the full canonical path
        /// form: `users/<name>`, `agents/<name>`, or
        /// `external/<system>/<name>`.
        #[arg(long, value_name = "ASSIGNEE", value_parser = parse_assignee_principal)]
        assignee: Option<Principal>,

        /// Description for the issue.
        #[arg(value_name = "DESCRIPTION")]
        description: String,

        /// Progress notes for the issue.
        #[arg(long, value_name = "PROGRESS")]
        progress: Option<String>,

        /// Issue id whose job settings will be used as defaults.
        #[arg(
            long = "current-issue-id",
            value_name = "ISSUE_ID",
            env = ENV_HYDRA_ISSUE_ID
        )]
        current_issue_id: Option<IssueId>,

        /// Repository name to use for job settings.
        #[arg(long = "repo-name", value_name = "REPO_NAME")]
        repo_name: Option<String>,

        /// Git remote URL to use for job settings.
        #[arg(long = "remote-url", value_name = "REMOTE_URL")]
        remote_url: Option<String>,

        /// Container image to use for job settings.
        #[arg(long, value_name = "IMAGE")]
        image: Option<String>,

        /// Model to use for job settings.
        #[arg(long, value_name = "MODEL")]
        model: Option<String>,

        /// Branch to use for job settings.
        #[arg(long, value_name = "BRANCH")]
        branch: Option<String>,

        /// Maximum retries to use for job settings.
        #[arg(long = "max-retries", value_name = "MAX_RETRIES")]
        max_retries: Option<u32>,

        /// User secrets to pass to jobs (comma-separated).
        #[arg(long, value_name = "SECRETS", value_delimiter = ',')]
        secrets: Vec<String>,

        /// Comma-separated label names to assign (creates labels if they don't exist).
        #[arg(long = "labels", value_name = "LABEL_NAME", value_delimiter = ',')]
        labels: Vec<String>,

        /// Path to a YAML file defining a form to attach to the issue.
        #[arg(long = "form", value_name = "FILE", conflicts_with = "form_inline")]
        form: Option<String>,

        /// Inline YAML string defining a form to attach to the issue.
        #[arg(long = "form-inline", value_name = "YAML")]
        form_inline: Option<String>,

        /// Feedback for the issue.
        #[arg(long, value_name = "FEEDBACK")]
        feedback: Option<String>,
    },
    /// Update an existing issue.
    Update {
        /// Issue ID to update.
        #[arg(value_name = "ISSUE_ID")]
        id: IssueId,

        /// New issue type.
        #[arg(long, value_name = "ISSUE_TYPE")]
        r#type: Option<IssueType>,

        /// Updated title.
        #[arg(long, value_name = "TITLE")]
        title: Option<String>,

        /// New issue status key. Validity depends on the issue's project;
        /// the server validates the value.
        #[arg(long, value_name = "STATUS_KEY")]
        status: Option<StatusKey>,

        /// Move the issue to a different project (by id or key), or clear
        /// it with `--clear-project`.
        #[arg(
            long,
            value_name = "PROJECT_ID_OR_KEY",
            conflicts_with = "clear_project"
        )]
        project: Option<ProjectRef>,

        /// Detach the issue from its current project (falls back to the
        /// default project's status semantics).
        #[arg(long = "clear-project")]
        clear_project: bool,

        /// Updated assignee. Requires the full canonical path form:
        /// `users/<name>`, `agents/<name>`, or `external/<system>/<name>`.
        #[arg(
            long,
            value_name = "ASSIGNEE",
            conflicts_with = "clear_assignee",
            value_parser = parse_assignee_principal,
        )]
        assignee: Option<Principal>,

        /// Remove the current assignee.
        #[arg(long)]
        clear_assignee: bool,

        /// Updated description.
        #[arg(long, value_name = "DESCRIPTION")]
        description: Option<String>,

        /// Replace dependencies with the provided set in the format TYPE:ISSUE_ID (e.g. child-of:i-abcd).
        #[arg(long = "deps", value_name = "TYPE:ISSUE_ID", value_parser = parse_issue_dependency, conflicts_with = "clear_dependencies")]
        dependencies: Vec<IssueDependency>,

        /// Remove all dependencies from the issue.
        #[arg(long)]
        clear_dependencies: bool,

        /// Replace the set of patches associated with the issue.
        #[arg(
            long = "patches",
            value_name = "PATCH_ID",
            value_delimiter = ',',
            conflicts_with = "clear_patches"
        )]
        patches: Vec<PatchId>,

        /// Remove all patches from the issue.
        #[arg(long)]
        clear_patches: bool,

        /// Updated progress notes.
        #[arg(long, value_name = "PROGRESS", conflicts_with = "clear_progress")]
        progress: Option<String>,

        /// Remove all progress notes from the issue.
        #[arg(long)]
        clear_progress: bool,

        /// Repository name to use for job settings.
        #[arg(
            long = "repo-name",
            value_name = "REPO_NAME",
            conflicts_with = "clear_job_settings"
        )]
        repo_name: Option<String>,

        /// Git remote URL to use for job settings.
        #[arg(
            long = "remote-url",
            value_name = "REMOTE_URL",
            conflicts_with = "clear_job_settings"
        )]
        remote_url: Option<String>,

        /// Container image to use for job settings.
        #[arg(long, value_name = "IMAGE", conflicts_with = "clear_job_settings")]
        image: Option<String>,

        /// Model to use for job settings.
        #[arg(long, value_name = "MODEL", conflicts_with = "clear_job_settings")]
        model: Option<String>,

        /// Branch to use for job settings.
        #[arg(long, value_name = "BRANCH", conflicts_with = "clear_job_settings")]
        branch: Option<String>,

        /// Maximum retries to use for job settings.
        #[arg(
            long = "max-retries",
            value_name = "MAX_RETRIES",
            conflicts_with = "clear_job_settings"
        )]
        max_retries: Option<u32>,

        /// User secrets to pass to jobs (comma-separated).
        #[arg(
            long,
            value_name = "SECRETS",
            value_delimiter = ',',
            conflicts_with_all = ["clear_job_settings", "clear_secrets"]
        )]
        secrets: Vec<String>,

        /// Remove secrets from job settings.
        #[arg(long, conflicts_with = "clear_job_settings")]
        clear_secrets: bool,

        /// Remove all job settings from the issue.
        #[arg(long)]
        clear_job_settings: bool,

        /// Comma-separated label names to add (creates labels if they don't exist).
        #[arg(long = "add-labels", value_name = "LABEL_NAME", value_delimiter = ',')]
        add_labels: Vec<String>,

        /// Comma-separated label names to remove.
        #[arg(
            long = "remove-labels",
            value_name = "LABEL_NAME",
            value_delimiter = ','
        )]
        remove_labels: Vec<String>,

        /// Path to a YAML file defining a form to set on the issue.
        #[arg(long = "form", value_name = "FILE", conflicts_with_all = ["form_inline", "clear_form"])]
        form: Option<String>,

        /// Inline YAML string defining a form to set on the issue.
        #[arg(
            long = "form-inline",
            value_name = "YAML",
            conflicts_with = "clear_form"
        )]
        form_inline: Option<String>,

        /// Remove the form from the issue.
        #[arg(long = "clear-form")]
        clear_form: bool,

        /// Updated feedback.
        #[arg(long, value_name = "FEEDBACK", conflicts_with = "clear_feedback")]
        feedback: Option<String>,

        /// Remove the current feedback.
        #[arg(long)]
        clear_feedback: bool,
    },
    /// Delete an issue.
    Delete {
        /// Issue ID to delete.
        #[arg(value_name = "ISSUE_ID")]
        id: IssueId,
    },
    /// Get the full details of a single issue by ID. Returns all fields including the complete description, progress notes, and job settings that are omitted from the summary returned by `list`.
    Get {
        /// Issue ID to get.
        #[arg(value_name = "ISSUE_ID")]
        id: IssueId,

        /// Include deleted issues in the result.
        #[arg(long = "include-deleted")]
        include_deleted: bool,

        /// Retrieve a specific version of the issue (positive = exact version, negative = offset from latest).
        #[arg(long)]
        version: Option<i64>,
    },
    /// Submit a form response for an issue.
    SubmitForm {
        /// Issue ID to submit the form for.
        #[arg(value_name = "ISSUE_ID")]
        id: IssueId,

        /// Action ID to take (must match an action defined on the issue's form).
        #[arg(long, value_name = "ACTION_ID")]
        action: String,

        /// Field values as a JSON or YAML string (object mapping field keys to values).
        #[arg(long, value_name = "JSON_OR_YAML", default_value = "{}")]
        values: String,
    },
}

pub async fn run(
    client: &dyn HydraClientInterface,
    command: IssueCommands,
    context: &CommandContext,
) -> Result<()> {
    match command {
        IssueCommands::List {
            id,
            r#type,
            status,
            project,
            assignee,
            query,
            labels,
            include_deleted,
        } => {
            let label_ids = resolve_label_names_to_ids(client, &labels).await?;
            let project_id = match project {
                Some(reference) => Some(reference.resolve(client).await?),
                None => None,
            };
            let issues = fetch_issues(
                client,
                id,
                r#type,
                status,
                project_id,
                assignee,
                query,
                label_ids,
                include_deleted,
            )
            .await?;
            write_issue_summary_records(context.output_format, &issues)?;
            Ok(())
        }
        IssueCommands::Create {
            r#type,
            title,
            status,
            project,
            dependencies,
            patches,
            assignee,
            description,
            progress,
            current_issue_id,
            repo_name,
            remote_url,
            image,
            model,
            branch,
            max_retries,
            secrets,
            labels,
            form,
            form_inline,
            feedback,
        } => {
            let parsed_form = parse_form_flag(form, form_inline)?;
            let creator = resolve_username(client).await?;
            let project_id = match project {
                Some(reference) => Some(reference.resolve(client).await?),
                None => None,
            };
            create_issue(
                client,
                r#type,
                title.unwrap_or_default(),
                status,
                project_id,
                dependencies,
                patches,
                assignee,
                creator,
                description,
                progress,
                repo_name,
                remote_url,
                image,
                model,
                branch,
                max_retries,
                secrets,
                current_issue_id,
                labels,
                parsed_form,
                feedback,
            )
            .await
            .and_then(|issue| write_issue_records(context.output_format, &[issue]))
        }
        IssueCommands::Update {
            id,
            r#type,
            title,
            status,
            project,
            clear_project,
            assignee,
            clear_assignee,
            description,
            dependencies,
            clear_dependencies,
            patches,
            clear_patches,
            progress,
            clear_progress,
            repo_name,
            remote_url,
            image,
            model,
            branch,
            max_retries,
            secrets,
            clear_secrets,
            clear_job_settings,
            add_labels,
            remove_labels,
            form,
            form_inline,
            clear_form,
            feedback,
            clear_feedback,
        } => {
            let parsed_form = parse_form_flag(form, form_inline)?;
            let project_update = if clear_project {
                ProjectUpdate::Clear
            } else if let Some(reference) = project {
                ProjectUpdate::Set(reference.resolve(client).await?)
            } else {
                ProjectUpdate::Unchanged
            };
            update_issue(
                client,
                id,
                r#type,
                title,
                status,
                project_update,
                assignee,
                clear_assignee,
                description,
                dependencies,
                clear_dependencies,
                patches,
                clear_patches,
                progress,
                clear_progress,
                repo_name,
                remote_url,
                image,
                model,
                branch,
                max_retries,
                secrets,
                clear_secrets,
                clear_job_settings,
                add_labels,
                remove_labels,
                parsed_form,
                clear_form,
                feedback,
                clear_feedback,
            )
            .await
            .and_then(|issue| write_issue_records(context.output_format, &[issue]))
        }
        IssueCommands::Delete { id } => {
            let deleted = client
                .delete_issue(&id)
                .await
                .with_context(|| format!("failed to delete issue '{id}'"))?;
            let mut buffer = Vec::new();
            render(
                DeletedIssueOutcome(&deleted.issue_id),
                context.output_format,
                &mut buffer,
            )?;
            write_stdout(&buffer)?;
            Ok(())
        }
        IssueCommands::Get {
            id,
            include_deleted,
            version,
        } => {
            let issue = resolve_issue(client, &id, include_deleted, version).await?;
            write_issue_records(context.output_format, &[issue])?;
            Ok(())
        }
        IssueCommands::SubmitForm { id, action, values } => {
            submit_form(client, id, action, values, context.output_format).await
        }
    }
}

/// Resolve a single issue, handling `--version` (positive, negative, or absent)
/// and `--include-deleted`.
async fn resolve_issue(
    client: &dyn HydraClientInterface,
    issue_id: &IssueId,
    include_deleted: bool,
    version: Option<i64>,
) -> Result<IssueVersionRecord> {
    match version {
        Some(0) => {
            bail!("--version 0 is not valid; use a positive version number or a negative offset")
        }
        Some(v) => client
            .get_issue_version(issue_id, RelativeVersionNumber::new(v))
            .await
            .with_context(|| format!("failed to fetch version {v} of issue '{issue_id}'")),
        None => client
            .get_issue(issue_id, include_deleted)
            .await
            .with_context(|| format!("failed to fetch issue '{issue_id}'")),
    }
}

async fn fetch_issues(
    client: &dyn HydraClientInterface,
    id: Option<IssueId>,
    issue_type: Option<IssueType>,
    status: Vec<StatusKey>,
    project_id: Option<ProjectId>,
    assignee: Option<Principal>,
    query: Option<String>,
    label_ids: Vec<LabelId>,
    include_deleted: bool,
) -> Result<Vec<IssueSummaryRecord>> {
    if let Some(issue_id) = id {
        let record = client
            .get_issue(&issue_id, include_deleted)
            .await
            .with_context(|| format!("failed to fetch issue '{issue_id}'"))?;

        if let Some(expected_type) = issue_type {
            if record.issue.issue_type != expected_type {
                bail!("Issue '{issue_id}' does not match the requested type.");
            }
        }
        if !status.is_empty() && !status.iter().any(|s| s == &record.issue.status) {
            bail!("Issue '{issue_id}' does not match the requested status.");
        }
        if let Some(ref expected_project) = project_id {
            if record.issue.project_id.as_ref() != Some(expected_project) {
                bail!("Issue '{issue_id}' does not belong to project {expected_project}.");
            }
        }
        if let Some(ref expected_assignee) = assignee {
            // Phase 4b: comparison is typed equality on Principal — no
            // ascii-case folding, kinds must match.
            if record.issue.assignee.as_ref() != Some(expected_assignee) {
                bail!("Issue '{issue_id}' is not assigned to {expected_assignee}.");
            }
        }
        return Ok(vec![IssueSummaryRecord::from(&record)]);
    }

    let trimmed_query = query.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    let include_deleted_opt = if include_deleted { Some(true) } else { None };
    let mut search_query = SearchIssuesQuery::new(
        issue_type,
        status.clone(),
        assignee.clone(),
        trimmed_query,
        include_deleted_opt,
    );
    search_query.project_id = project_id.clone();
    search_query.label_ids = label_ids;
    let issues = client
        .list_issues(&search_query)
        .await
        .context("failed to list issues")?
        .issues;

    for issue in &issues {
        if let Some(expected_type) = issue_type {
            if issue.issue.issue_type != expected_type {
                bail!(
                    "Issue {} does not match the requested type.",
                    issue.issue_id
                );
            }
        }
        if !status.is_empty() && !status.iter().any(|s| s == &issue.issue.status) {
            bail!(
                "Issue {} does not match the requested status.",
                issue.issue_id
            );
        }
        if let Some(ref expected_project) = project_id {
            if issue.issue.project_id.as_ref() != Some(expected_project) {
                bail!(
                    "Issue {} does not belong to project {expected_project}.",
                    issue.issue_id
                );
            }
        }
        if let Some(ref expected_assignee) = assignee {
            if issue.issue.assignee.as_ref() != Some(expected_assignee) {
                bail!(
                    "Issue {} is not assigned to {expected_assignee}",
                    issue.issue_id
                );
            }
        }
    }

    Ok(issues)
}

fn resolve_job_settings(
    current: SessionSettings,
    repo_name: Option<String>,
    remote_url: Option<String>,
    image: Option<String>,
    model: Option<String>,
    branch: Option<String>,
    max_retries: Option<u32>,
    secrets: Vec<String>,
    clear_secrets: bool,
    clear_job_settings: bool,
) -> Result<(SessionSettings, bool)> {
    if clear_job_settings {
        return Ok((SessionSettings::default(), true));
    }

    let mut changed = false;
    let mut job_settings = current.clone();

    if let Some(value) = repo_name {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            bail!("--repo-name must not be empty when provided.");
        }
        let parsed = RepoName::from_str(trimmed)
            .map_err(|err| anyhow!("invalid repo name '{trimmed}': {err}"))?;
        job_settings.repo_name = Some(parsed);
        changed = true;
    }

    if let Some(value) = remote_url {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            bail!("--remote-url must not be empty when provided.");
        }
        job_settings.remote_url = Some(trimmed.to_string());
        changed = true;
    }

    if let Some(value) = image {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            bail!("--image must not be empty when provided.");
        }
        job_settings.image = Some(trimmed.to_string());
        changed = true;
    }

    if let Some(value) = model {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            bail!("--model must not be empty when provided.");
        }
        job_settings.model = Some(trimmed.to_string());
        changed = true;
    }

    if let Some(value) = branch {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            bail!("--branch must not be empty when provided.");
        }
        job_settings.branch = Some(trimmed.to_string());
        changed = true;
    }

    if let Some(value) = max_retries {
        job_settings.max_retries = Some(value);
        changed = true;
    }

    if clear_secrets {
        job_settings.secrets = None;
        changed = true;
    } else if !secrets.is_empty() {
        let validated: Vec<String> = secrets
            .into_iter()
            .map(|s| {
                let trimmed = s.trim().to_string();
                if trimmed.is_empty() {
                    bail!("--secrets values must not be empty.");
                }
                Ok(trimmed)
            })
            .collect::<Result<Vec<_>>>()?;
        job_settings.secrets = Some(validated);
        changed = true;
    }

    if changed {
        Ok((job_settings, true))
    } else {
        Ok((current, false))
    }
}

async fn resolve_inherited_job_settings(
    client: &dyn HydraClientInterface,
    current_issue_id: Option<IssueId>,
) -> Result<SessionSettings> {
    let Some(issue_id) = current_issue_id else {
        return Ok(SessionSettings::default());
    };

    let issue = client
        .get_issue(&issue_id, false)
        .await
        .with_context(|| format!("failed to fetch issue '{issue_id}'"))?;

    let mut job_settings = SessionSettings::default();
    let current = issue.issue.session_settings;
    job_settings.repo_name = current.repo_name;
    job_settings.remote_url = current.remote_url;
    job_settings.image = current.image;
    job_settings.model = current.model;
    job_settings.branch = current.branch;
    job_settings.secrets = current.secrets;

    Ok(job_settings)
}

/// Parse a form from either a `--form <file>` or `--form-inline <yaml>` flag.
fn parse_form_flag(form_file: Option<String>, form_inline: Option<String>) -> Result<Option<Form>> {
    let yaml_str = match (form_file, form_inline) {
        (Some(path), None) => std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read form file '{path}'"))?,
        (None, Some(inline)) => inline,
        (None, None) => return Ok(None),
        _ => unreachable!("clap conflicts_with prevents both being set"),
    };

    let form: Form = serde_yaml_ng::from_str(&yaml_str).context("failed to parse form YAML")?;
    form.validate_field_keys()
        .map_err(|e| anyhow!("invalid form: {e}"))?;
    Ok(Some(form))
}

/// Parse values from a JSON or YAML string into a map.
fn parse_values(values_str: &str) -> Result<std::collections::HashMap<String, serde_json::Value>> {
    // Try JSON first, then YAML
    if let Ok(map) = serde_json::from_str(values_str) {
        return Ok(map);
    }
    serde_yaml_ng::from_str(values_str).context("failed to parse values as JSON or YAML")
}

async fn submit_form(
    client: &dyn HydraClientInterface,
    id: IssueId,
    action: String,
    values: String,
    output_format: ResolvedOutputFormat,
) -> Result<()> {
    let values = parse_values(&values)?;
    let request = SubmitFormRequest::new(action, values);
    let response = client
        .submit_form(&id, &request)
        .await
        .with_context(|| format!("failed to submit form for issue '{id}'"))?;

    let mut buf = Vec::new();
    render(SubmitFormOutcome(&response), output_format, &mut buf)?;
    write_stdout(&buf)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn create_issue(
    client: &dyn HydraClientInterface,
    issue_type: IssueType,
    title: String,
    status: StatusKey,
    project_id: Option<ProjectId>,
    dependencies: Vec<IssueDependency>,
    patches: Vec<PatchId>,
    assignee: Option<Principal>,
    creator: Username,
    description: String,
    progress: Option<String>,
    repo_name: Option<String>,
    remote_url: Option<String>,
    image: Option<String>,
    model: Option<String>,
    branch: Option<String>,
    max_retries: Option<u32>,
    secrets: Vec<String>,
    current_issue_id: Option<IssueId>,
    labels: Vec<String>,
    form: Option<Form>,
    feedback: Option<String>,
) -> Result<IssueVersionRecord> {
    let description = description.trim();
    if description.is_empty() {
        bail!("Issue description must not be empty.");
    }

    let progress = progress
        .map(|value| value.trim().to_string())
        .unwrap_or_default();

    let inherited_job_settings = resolve_inherited_job_settings(client, current_issue_id).await?;

    let (job_settings, job_settings_requested) = resolve_job_settings(
        inherited_job_settings,
        repo_name,
        remote_url,
        image,
        model,
        branch,
        max_retries,
        secrets,
        false,
        false,
    )?;
    let job_settings = (job_settings_requested || !SessionSettings::is_default(&job_settings))
        .then_some(job_settings);

    let issue = Issue::new(
        issue_type,
        title,
        description.to_string(),
        creator,
        progress,
        status,
        project_id,
        assignee,
        job_settings,
        dependencies,
        patches,
        false,
        form,
        None,
        feedback,
    );
    let mut request = UpsertIssueRequest::new(issue.clone(), None);
    let label_names: Vec<String> = labels
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if !label_names.is_empty() {
        request.label_names = Some(label_names);
    }

    let response = client
        .create_issue(&request)
        .await
        .context("failed to create issue")?;

    Ok(IssueVersionRecord::new(
        response.issue_id,
        response.version,
        Utc::now(),
        issue,
        None,
        Utc::now(),
        Vec::new(),
    ))
}

#[allow(clippy::too_many_arguments)]
async fn update_issue(
    client: &dyn HydraClientInterface,
    id: IssueId,
    issue_type: Option<IssueType>,
    title: Option<String>,
    status: Option<StatusKey>,
    project_update: ProjectUpdate,
    assignee: Option<Principal>,
    clear_assignee: bool,
    description: Option<String>,
    dependencies: Vec<IssueDependency>,
    clear_dependencies: bool,
    patches: Vec<PatchId>,
    clear_patches: bool,
    progress: Option<String>,
    clear_progress: bool,
    repo_name: Option<String>,
    remote_url: Option<String>,
    image: Option<String>,
    model: Option<String>,
    branch: Option<String>,
    max_retries: Option<u32>,
    secrets: Vec<String>,
    clear_secrets: bool,
    clear_job_settings: bool,
    add_labels: Vec<String>,
    remove_labels: Vec<String>,
    form: Option<Form>,
    clear_form: bool,
    feedback: Option<String>,
    clear_feedback: bool,
) -> Result<IssueVersionRecord> {
    let issue_id = id;

    let description = match description {
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                bail!("Issue description must not be empty.");
            }
            Some(trimmed.to_string())
        }
        None => None,
    };

    let assignee = if clear_assignee {
        Some(None)
    } else {
        assignee.map(Some)
    };

    let dependencies_update = if clear_dependencies {
        Some(Vec::new())
    } else if dependencies.is_empty() {
        None
    } else {
        Some(dependencies)
    };

    let patches_update = if clear_patches {
        Some(Vec::new())
    } else if patches.is_empty() {
        None
    } else {
        Some(patches)
    };

    let progress_update = if clear_progress {
        Some(String::new())
    } else {
        progress.map(|value| value.trim().to_string())
    };

    let feedback_update = if clear_feedback {
        Some(None)
    } else {
        feedback.map(|f| {
            let trimmed = f.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        })
    };

    let job_settings_requested = clear_job_settings
        || repo_name.is_some()
        || remote_url.is_some()
        || image.is_some()
        || model.is_some()
        || branch.is_some()
        || max_retries.is_some()
        || !secrets.is_empty()
        || clear_secrets;

    let labels_requested = !add_labels.is_empty() || !remove_labels.is_empty();
    let form_requested = form.is_some() || clear_form;

    let no_changes = issue_type.is_none()
        && title.is_none()
        && status.is_none()
        && matches!(project_update, ProjectUpdate::Unchanged)
        && assignee.is_none()
        && description.is_none()
        && dependencies_update.is_none()
        && patches_update.is_none()
        && progress_update.is_none()
        && feedback_update.is_none()
        && !job_settings_requested
        && !labels_requested
        && !form_requested;
    if no_changes {
        bail!("At least one field must be provided to update.");
    }

    let current = client
        .get_issue(&issue_id, false)
        .await
        .with_context(|| format!("failed to fetch issue '{issue_id}'"))?;

    let (job_settings, job_settings_changed) = resolve_job_settings(
        current.issue.session_settings.clone(),
        repo_name,
        remote_url,
        image,
        model,
        branch,
        max_retries,
        secrets,
        clear_secrets,
        clear_job_settings,
    )?;
    let job_settings = if job_settings_changed {
        Some(job_settings)
    } else {
        Some(current.issue.session_settings.clone())
    };

    let form_update = if clear_form {
        Some(None)
    } else {
        form.map(Some)
    };

    let issue_fields_changed = issue_type.is_some()
        || title.is_some()
        || status.is_some()
        || !matches!(project_update, ProjectUpdate::Unchanged)
        || assignee.is_some()
        || description.is_some()
        || dependencies_update.is_some()
        || patches_update.is_some()
        || progress_update.is_some()
        || feedback_update.is_some()
        || job_settings_changed
        || form_update.is_some();

    let result = if issue_fields_changed {
        let project_id = match project_update {
            ProjectUpdate::Unchanged => current.issue.project_id,
            ProjectUpdate::Set(id) => Some(id),
            ProjectUpdate::Clear => None,
        };
        let updated_issue = Issue::new(
            issue_type.unwrap_or(current.issue.issue_type),
            title.unwrap_or(current.issue.title),
            description.unwrap_or(current.issue.description),
            current.issue.creator,
            progress_update.unwrap_or(current.issue.progress),
            status.unwrap_or(current.issue.status),
            project_id,
            assignee.unwrap_or(current.issue.assignee),
            job_settings,
            dependencies_update.unwrap_or(current.issue.dependencies),
            patches_update.unwrap_or(current.issue.patches),
            current.issue.deleted,
            form_update.unwrap_or(current.issue.form),
            current.issue.form_response,
            feedback_update.unwrap_or(current.issue.feedback),
        );

        let response = client
            .update_issue(
                &issue_id,
                &UpsertIssueRequest::new(updated_issue.clone(), None),
            )
            .await
            .with_context(|| format!("failed to update issue '{issue_id}'"))?;

        IssueVersionRecord::new(
            response.issue_id,
            response.version,
            Utc::now(),
            updated_issue,
            None,
            Utc::now(),
            Vec::new(),
        )
    } else {
        IssueVersionRecord::new(
            issue_id.clone(),
            current.version,
            current.timestamp,
            current.issue,
            current.actor,
            current.creation_time,
            current.labels.clone(),
        )
    };

    if labels_requested {
        let object_id = HydraId::from(issue_id.clone());
        apply_label_changes(client, &object_id, &add_labels, &remove_labels).await?;

        // Re-fetch the issue to get fresh label data after changes.
        let refreshed = client
            .get_issue(&issue_id, false)
            .await
            .with_context(|| format!("failed to re-fetch issue '{issue_id}' after label update"))?;
        return Ok(refreshed);
    }

    Ok(result)
}

fn write_issue_records(format: ResolvedOutputFormat, issues: &[IssueVersionRecord]) -> Result<()> {
    let mut buffer = Vec::new();
    render(IssueRecords(issues), format, &mut buffer)?;
    write_stdout(&buffer)?;
    Ok(())
}

fn write_issue_summary_records(
    format: ResolvedOutputFormat,
    issues: &[IssueSummaryRecord],
) -> Result<()> {
    let mut buffer = Vec::new();
    render(IssueSummaryRecords(issues), format, &mut buffer)?;
    write_stdout(&buffer)?;
    Ok(())
}

/// Resolve a list of label names to their IDs by querying the server.
/// Returns an error if any label name cannot be found.
async fn resolve_label_names_to_ids(
    client: &dyn HydraClientInterface,
    names: &[String],
) -> Result<Vec<LabelId>> {
    if names.is_empty() {
        return Ok(Vec::new());
    }

    let mut ids = Vec::with_capacity(names.len());
    for name in names {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut query = SearchLabelsQuery::default();
        query.q = Some(trimmed.to_string());
        let response = client
            .list_labels(&query)
            .await
            .context("failed to list labels")?;
        let label = response
            .labels
            .iter()
            .find(|l| l.name.eq_ignore_ascii_case(trimmed))
            .ok_or_else(|| anyhow!("label '{trimmed}' not found"))?;
        ids.push(label.label_id.clone());
    }
    Ok(ids)
}

/// Resolve a label name to its ID, creating it if it doesn't exist.
async fn resolve_or_create_label(
    client: &dyn HydraClientInterface,
    name: &str,
    all_labels: &[hydra_common::api::v1::labels::LabelRecord],
) -> Result<LabelId> {
    let normalized = name.trim().to_lowercase();
    if let Some(label) = all_labels
        .iter()
        .find(|l| l.name.to_lowercase() == normalized)
    {
        return Ok(label.label_id.clone());
    }
    let request = UpsertLabelRequest::new(Label::new(name.trim().to_string(), None));
    let response = client
        .create_label(&request)
        .await
        .with_context(|| format!("failed to create label '{}'", name.trim()))?;
    Ok(response.label_id)
}

/// Add and remove label associations for an object.
async fn apply_label_changes(
    client: &dyn HydraClientInterface,
    object_id: &HydraId,
    add_labels: &[String],
    remove_labels: &[String],
) -> Result<()> {
    if add_labels.is_empty() && remove_labels.is_empty() {
        return Ok(());
    }

    let all_labels = client
        .list_labels(&SearchLabelsQuery::default())
        .await
        .context("failed to list labels")?
        .labels;

    for name in add_labels {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            continue;
        }
        let label_id = resolve_or_create_label(client, trimmed, &all_labels).await?;
        client
            .add_label_association(&label_id, object_id)
            .await
            .with_context(|| format!("failed to add label '{trimmed}' to object"))?;
    }

    for name in remove_labels {
        let trimmed = name.trim().to_lowercase();
        if trimmed.is_empty() {
            continue;
        }
        let label = all_labels
            .iter()
            .find(|l| l.name.to_lowercase() == trimmed)
            .ok_or_else(|| anyhow!("label '{}' not found", name.trim()))?;
        client
            .remove_label_association(&label.label_id, object_id)
            .await
            .with_context(|| format!("failed to remove label '{}' from object", name.trim()))?;
    }

    Ok(())
}

fn parse_issue_dependency(raw: &str) -> Result<IssueDependency, String> {
    let (dependency_type, issue_id) = raw
        .split_once(':')
        .ok_or_else(|| "dependency must be in the format TYPE:ISSUE_ID".to_string())?;

    let dependency_type =
        IssueDependencyType::from_str(dependency_type).map_err(|err| err.to_string())?;
    let issue_id = issue_id
        .trim()
        .parse::<IssueId>()
        .map_err(|err| err.to_string())?;
    Ok(IssueDependency::new(dependency_type, issue_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::HydraClient;
    use crate::test_utils::ids::{issue_id, patch_id};
    use chrono::Utc;
    use httpmock::prelude::*;
    use hydra_common::issues::{
        Issue, IssueStatus, IssueSummaryRecord, IssueVersionRecord, ListIssuesResponse,
        SessionSettings, UpsertIssueRequest, UpsertIssueResponse,
    };
    use hydra_common::{users::Username, PatchId, RepoName};
    use reqwest::Client as HttpClient;
    use std::str::FromStr;

    const TEST_HYDRA_TOKEN: &str = "test-hydra-token";

    fn sample_repo_name() -> RepoName {
        RepoName::from_str("dourolabs/example").unwrap()
    }

    fn sample_job_settings() -> SessionSettings {
        let mut job_settings = SessionSettings::default();
        job_settings.repo_name = Some(sample_repo_name());
        job_settings.remote_url = Some("https://example.com/service.git".into());
        job_settings.image = Some("worker:123".into());
        job_settings.model = Some("gpt-4o".into());
        job_settings.branch = Some("main".into());
        job_settings.max_retries = Some(5);
        job_settings.cpu_limit = Some("750m".into());
        job_settings.memory_limit = Some("2Gi".into());
        job_settings
    }

    fn empty_user() -> Username {
        Username::from("")
    }

    fn hydra_client(server: &MockServer) -> HydraClient {
        HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())
            .unwrap()
    }

    /// Construct a `Principal::User` from a `&str` for test fixtures.
    /// Phase 4b's API surface is typed; callers that used to pass a bare
    /// string now pass `Principal::User { name }` via this helper.
    fn user_principal(name: &str) -> Principal {
        Principal::User {
            name: hydra_common::api::v1::users::Username::try_new(name)
                .expect("test username should validate"),
        }
    }

    fn api_issue_record(
        id: &str,
        issue_type: IssueType,
        description: &str,
        status: IssueStatus,
        assignee: Option<&str>,
        dependencies: Vec<IssueDependency>,
        patches: Vec<PatchId>,
    ) -> IssueVersionRecord {
        IssueVersionRecord::new(
            issue_id(id),
            0,
            Utc::now(),
            Issue::new(
                issue_type,
                "Test Title".to_string(),
                description.into(),
                empty_user(),
                String::new(),
                status.into(),
                None,
                assignee.map(user_principal),
                None,
                dependencies,
                patches,
                false,
                None,
                None,
                None,
            ),
            None,
            Utc::now(),
            Vec::new(),
        )
    }

    #[tokio::test]
    async fn list_issues_filters_by_query_and_prints_jsonl() {
        let server = MockServer::start();
        let client = hydra_client(&server);
        let issues_response =
            ListIssuesResponse::new(vec![IssueSummaryRecord::from(&IssueVersionRecord::new(
                issue_id("i-1"),
                0,
                Utc::now(),
                Issue::new(
                    IssueType::Bug,
                    "Test Title".to_string(),
                    "First issue".into(),
                    empty_user(),
                    String::new(),
                    IssueStatus::Open.into(),
                    None,
                    None,
                    None,
                    vec![],
                    Vec::new(),
                    false,
                    None,
                    None,
                    None,
                ),
                None,
                Utc::now(),
                Vec::new(),
            ))]);
        let list_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/issues")
                .query_param("issue_type", IssueType::Bug.as_str())
                .query_param("status", IssueStatus::Open.as_str())
                .query_param("q", "bug");
            then.status(200).json_body_obj(&issues_response);
        });

        let issues = fetch_issues(
            &client,
            None,
            Some(IssueType::Bug),
            vec![IssueStatus::Open.as_status_key()],
            None,
            None,
            Some("bug".into()),
            Vec::new(),
            false,
        )
        .await
        .unwrap();

        list_mock.assert();
        assert_eq!(list_mock.hits(), 1);

        let mut output = Vec::new();
        render(
            IssueSummaryRecords(&issues),
            ResolvedOutputFormat::Jsonl,
            &mut output,
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        let first_id = issue_id("i-1").to_string();
        let second_id = issue_id("i-2").to_string();
        assert!(output.contains(&format!("\"issue_id\":\"{first_id}\"")));
        assert!(!output.contains(&format!("\"issue_id\":\"{second_id}\"")));
    }

    #[tokio::test]
    async fn list_issues_by_id_returns_single_issue() {
        let server = MockServer::start();
        let client = hydra_client(&server);
        let issue_id = issue_id("i-123");
        let issue_record = IssueVersionRecord::new(
            issue_id.clone(),
            0,
            Utc::now(),
            Issue::new(
                IssueType::Task,
                "Test Title".to_string(),
                "Edge case bug".into(),
                empty_user(),
                String::new(),
                IssueStatus::InProgress.into(),
                None,
                None,
                None,
                vec![],
                Vec::new(),
                false,
                None,
                None,
                None,
            ),
            None,
            Utc::now(),
            Vec::new(),
        );
        let get_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{issue_id}").as_str());
            then.status(200).json_body_obj(&issue_record);
        });

        let issues = fetch_issues(
            &client,
            Some(issue_id.clone()),
            Some(IssueType::Task),
            vec![IssueStatus::InProgress.as_status_key()],
            None,
            None,
            None,
            Vec::new(),
            false,
        )
        .await
        .unwrap();

        get_mock.assert();
        assert_eq!(get_mock.hits(), 1);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].issue_id, issue_id);
    }

    #[tokio::test]
    async fn list_issues_filters_by_assignee() {
        let server = MockServer::start();
        let client = hydra_client(&server);
        let issues_response =
            ListIssuesResponse::new(vec![IssueSummaryRecord::from(&IssueVersionRecord::new(
                issue_id("i-7"),
                0,
                Utc::now(),
                Issue::new(
                    IssueType::Task,
                    "Test Title".to_string(),
                    "Edge case bug".into(),
                    empty_user(),
                    String::new(),
                    IssueStatus::Open.into(),
                    None,
                    Some(user_principal("owner-a")),
                    None,
                    vec![],
                    Vec::new(),
                    false,
                    None,
                    None,
                    None,
                ),
                None,
                Utc::now(),
                Vec::new(),
            ))]);
        let list_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/issues")
                .query_param("assignee", "users/owner-a");
            then.status(200).json_body_obj(&issues_response);
        });

        let issues = fetch_issues(
            &client,
            None,
            None,
            Vec::new(),
            None,
            Some(user_principal("owner-a")),
            None,
            Vec::new(),
            false,
        )
        .await
        .unwrap();

        list_mock.assert();
        assert_eq!(list_mock.hits(), 1);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].issue_id, issue_id("i-7"));
    }

    #[tokio::test]
    async fn create_issue_submits_issue_record() {
        let server = MockServer::start();
        let client = hydra_client(&server);

        let patch_ids = vec![patch_id("p-123")];
        let create_request = UpsertIssueRequest::new(
            Issue::new(
                IssueType::MergeRequest,
                "Test Title".to_string(),
                "New issue description".into(),
                Username::from("creator-a"),
                "Initial notes".into(),
                IssueStatus::Closed.into(),
                None,
                Some(user_principal("team-a")),
                None,
                Vec::new(),
                patch_ids.clone(),
                false,
                None,
                None,
                None,
            ),
            None,
        );
        let create_mock = server.mock(|when, then| {
            when.method(POST)
                .path("/v1/issues")
                .json_body_obj(&create_request);
            then.status(200)
                .json_body_obj(&UpsertIssueResponse::new(issue_id("i-456"), 0));
        });

        create_issue(
            &client,
            IssueType::MergeRequest,
            "Test Title".to_string(),
            IssueStatus::Closed.into(),
            None,
            Vec::new(),
            patch_ids.clone(),
            Some(user_principal("team-a")),
            Username::from("creator-a"),
            "New issue description".into(),
            Some("Initial notes".into()),
            None,
            None,
            None,
            None,
            None,
            None,
            Vec::new(),
            None,
            Vec::new(),
            None,
            None,
        )
        .await
        .unwrap();

        create_mock.assert();
        assert_eq!(create_mock.hits(), 1);
    }

    #[tokio::test]
    async fn create_issue_sets_job_settings() {
        let server = MockServer::start();
        let client = hydra_client(&server);

        let mut job_settings = SessionSettings::default();
        job_settings.repo_name = Some(sample_repo_name());
        job_settings.remote_url = Some("https://example.com/service.git".into());
        job_settings.image = Some("worker:latest".into());
        job_settings.branch = Some("feature/job-settings".into());
        job_settings.max_retries = Some(4);
        let create_request = UpsertIssueRequest::new(
            Issue::new(
                IssueType::MergeRequest,
                "Test Title".to_string(),
                "New issue description".into(),
                Username::from("creator-a"),
                "Initial notes".into(),
                IssueStatus::Closed.into(),
                None,
                Some(user_principal("team-a")),
                Some(job_settings.clone()),
                Vec::new(),
                vec![],
                false,
                None,
                None,
                None,
            ),
            None,
        );
        let create_mock = server.mock(|when, then| {
            when.method(POST)
                .path("/v1/issues")
                .json_body_obj(&create_request);
            then.status(200)
                .json_body_obj(&UpsertIssueResponse::new(issue_id("i-456"), 0));
        });

        create_issue(
            &client,
            IssueType::MergeRequest,
            "Test Title".to_string(),
            IssueStatus::Closed.into(),
            None,
            Vec::new(),
            vec![],
            Some(user_principal("team-a")),
            Username::from("creator-a"),
            "New issue description".into(),
            Some("Initial notes".into()),
            Some("dourolabs/example".into()),
            Some("https://example.com/service.git".into()),
            Some("worker:latest".into()),
            None,
            Some("feature/job-settings".into()),
            Some(4),
            Vec::new(),
            None,
            Vec::new(),
            None,
            None,
        )
        .await
        .unwrap();

        create_mock.assert();
        assert_eq!(create_mock.hits(), 1);
    }

    #[tokio::test]
    async fn create_issue_inherits_job_settings_from_current_issue() {
        let server = MockServer::start();
        let client = hydra_client(&server);

        let current_issue_id = issue_id("i-current");
        let mut inherited_settings = SessionSettings::default();
        inherited_settings.repo_name = Some(sample_repo_name());
        inherited_settings.remote_url = Some("https://example.com/service.git".into());
        inherited_settings.image = Some("worker:latest".into());
        inherited_settings.branch = Some("feature/job-settings".into());
        let current_issue = IssueVersionRecord::new(
            current_issue_id.clone(),
            0,
            Utc::now(),
            Issue::new(
                IssueType::Task,
                "Test Title".to_string(),
                "Existing issue".into(),
                empty_user(),
                String::new(),
                IssueStatus::Open.into(),
                None,
                None,
                Some(inherited_settings.clone()),
                Vec::new(),
                Vec::new(),
                false,
                None,
                None,
                None,
            ),
            None,
            Utc::now(),
            Vec::new(),
        );
        let current_issue_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{current_issue_id}").as_str());
            then.status(200).json_body_obj(&current_issue);
        });
        let create_request = UpsertIssueRequest::new(
            Issue::new(
                IssueType::MergeRequest,
                "Test Title".to_string(),
                "New issue description".into(),
                Username::from("creator-a"),
                "Initial notes".into(),
                IssueStatus::Open.into(),
                None,
                None,
                Some(inherited_settings),
                Vec::new(),
                vec![],
                false,
                None,
                None,
                None,
            ),
            None,
        );
        let create_mock = server.mock(|when, then| {
            when.method(POST)
                .path("/v1/issues")
                .json_body_obj(&create_request);
            then.status(200)
                .json_body_obj(&UpsertIssueResponse::new(issue_id("i-new"), 0));
        });

        create_issue(
            &client,
            IssueType::MergeRequest,
            "Test Title".to_string(),
            IssueStatus::Open.into(),
            None,
            Vec::new(),
            vec![],
            None,
            Username::from("creator-a"),
            "New issue description".into(),
            Some("Initial notes".into()),
            None,
            None,
            None,
            None,
            None,
            None,
            Vec::new(),
            Some(current_issue_id),
            Vec::new(),
            None,
            None,
        )
        .await
        .unwrap();

        current_issue_mock.assert();
        create_mock.assert();
        assert_eq!(current_issue_mock.hits(), 1);
        assert_eq!(create_mock.hits(), 1);
    }

    #[tokio::test]
    async fn create_issue_overrides_inherited_job_settings() {
        let server = MockServer::start();
        let client = hydra_client(&server);

        let current_issue_id = issue_id("i-current");
        let mut inherited_settings = SessionSettings::default();
        inherited_settings.repo_name = Some(sample_repo_name());
        inherited_settings.remote_url = Some("https://example.com/service.git".into());
        inherited_settings.image = Some("worker:latest".into());
        inherited_settings.branch = Some("feature/job-settings".into());
        let current_issue = IssueVersionRecord::new(
            current_issue_id.clone(),
            0,
            Utc::now(),
            Issue::new(
                IssueType::Task,
                "Test Title".to_string(),
                "Existing issue".into(),
                empty_user(),
                String::new(),
                IssueStatus::Open.into(),
                None,
                None,
                Some(inherited_settings.clone()),
                Vec::new(),
                Vec::new(),
                false,
                None,
                None,
                None,
            ),
            None,
            Utc::now(),
            Vec::new(),
        );
        let current_issue_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{current_issue_id}").as_str());
            then.status(200).json_body_obj(&current_issue);
        });

        let mut expected_settings = SessionSettings::default();
        expected_settings.repo_name = Some(RepoName::from_str("dourolabs/override").unwrap());
        expected_settings.remote_url = inherited_settings.remote_url.clone();
        expected_settings.image = Some("custom:tag".into());
        expected_settings.branch = Some("override-branch".into());
        let create_request = UpsertIssueRequest::new(
            Issue::new(
                IssueType::MergeRequest,
                "Test Title".to_string(),
                "New issue description".into(),
                Username::from("creator-a"),
                "Initial notes".into(),
                IssueStatus::Open.into(),
                None,
                None,
                Some(expected_settings.clone()),
                Vec::new(),
                vec![],
                false,
                None,
                None,
                None,
            ),
            None,
        );
        let create_mock = server.mock(|when, then| {
            when.method(POST)
                .path("/v1/issues")
                .json_body_obj(&create_request);
            then.status(200)
                .json_body_obj(&UpsertIssueResponse::new(issue_id("i-new"), 0));
        });

        create_issue(
            &client,
            IssueType::MergeRequest,
            "Test Title".to_string(),
            IssueStatus::Open.into(),
            None,
            Vec::new(),
            vec![],
            None,
            Username::from("creator-a"),
            "New issue description".into(),
            Some("Initial notes".into()),
            Some("dourolabs/override".into()),
            None,
            Some("custom:tag".into()),
            None,
            Some("override-branch".into()),
            None,
            Vec::new(),
            Some(current_issue_id),
            Vec::new(),
            None,
            None,
        )
        .await
        .unwrap();

        current_issue_mock.assert();
        create_mock.assert();
        assert_eq!(current_issue_mock.hits(), 1);
        assert_eq!(create_mock.hits(), 1);
    }

    #[tokio::test]
    async fn create_issue_sets_secrets() {
        let server = MockServer::start();
        let client = hydra_client(&server);

        let mut job_settings = SessionSettings::default();
        job_settings.secrets = Some(vec!["my-api-secret".into(), "my-db-secret".into()]);
        let create_request = UpsertIssueRequest::new(
            Issue::new(
                IssueType::Task,
                "Test Title".to_string(),
                "Issue with secrets".into(),
                Username::from("creator-a"),
                String::new(),
                IssueStatus::Open.into(),
                None,
                None,
                Some(job_settings.clone()),
                Vec::new(),
                vec![],
                false,
                None,
                None,
                None,
            ),
            None,
        );
        let create_mock = server.mock(|when, then| {
            when.method(POST)
                .path("/v1/issues")
                .json_body_obj(&create_request);
            then.status(200)
                .json_body_obj(&UpsertIssueResponse::new(issue_id("i-secrets"), 0));
        });

        create_issue(
            &client,
            IssueType::Task,
            "Test Title".to_string(),
            IssueStatus::Open.into(),
            None,
            Vec::new(),
            vec![],
            None,
            Username::from("creator-a"),
            "Issue with secrets".into(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            vec!["my-api-secret".into(), "my-db-secret".into()],
            None,
            Vec::new(),
            None,
            None,
        )
        .await
        .unwrap();

        create_mock.assert();
        assert_eq!(create_mock.hits(), 1);
    }

    #[tokio::test]
    async fn create_issue_inherits_secrets_from_current_issue() {
        let server = MockServer::start();
        let client = hydra_client(&server);

        let current_issue_id = issue_id("i-current");
        let mut inherited_settings = SessionSettings::default();
        inherited_settings.secrets = Some(vec!["inherited-secret".into()]);
        let current_issue = IssueVersionRecord::new(
            current_issue_id.clone(),
            0,
            Utc::now(),
            Issue::new(
                IssueType::Task,
                "Test Title".to_string(),
                "Parent issue".into(),
                empty_user(),
                String::new(),
                IssueStatus::Open.into(),
                None,
                None,
                Some(inherited_settings.clone()),
                Vec::new(),
                Vec::new(),
                false,
                None,
                None,
                None,
            ),
            None,
            Utc::now(),
            Vec::new(),
        );
        let current_issue_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{current_issue_id}").as_str());
            then.status(200).json_body_obj(&current_issue);
        });
        let create_request = UpsertIssueRequest::new(
            Issue::new(
                IssueType::Task,
                "Test Title".to_string(),
                "Child issue".into(),
                Username::from("creator-a"),
                String::new(),
                IssueStatus::Open.into(),
                None,
                None,
                Some(inherited_settings),
                Vec::new(),
                vec![],
                false,
                None,
                None,
                None,
            ),
            None,
        );
        let create_mock = server.mock(|when, then| {
            when.method(POST)
                .path("/v1/issues")
                .json_body_obj(&create_request);
            then.status(200)
                .json_body_obj(&UpsertIssueResponse::new(issue_id("i-child"), 0));
        });

        create_issue(
            &client,
            IssueType::Task,
            "Test Title".to_string(),
            IssueStatus::Open.into(),
            None,
            Vec::new(),
            vec![],
            None,
            Username::from("creator-a"),
            "Child issue".into(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Vec::new(),
            Some(current_issue_id),
            Vec::new(),
            None,
            None,
        )
        .await
        .unwrap();

        current_issue_mock.assert();
        create_mock.assert();
        assert_eq!(current_issue_mock.hits(), 1);
        assert_eq!(create_mock.hits(), 1);
    }

    #[tokio::test]
    async fn create_issue_requires_description() {
        let server = MockServer::start();
        let client = hydra_client(&server);
        assert!(create_issue(
            &client,
            IssueType::Bug,
            "Test Title".to_string(),
            IssueStatus::Open.into(),
            None,
            vec![],
            Vec::new(),
            None,
            empty_user(),
            "   ".into(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Vec::new(),
            None,
            Vec::new(),
            None,
            None,
        )
        .await
        .is_err());
    }

    #[test]
    fn parse_assignee_principal_rejects_bare_username_with_hint() {
        let err = parse_assignee_principal("alice").unwrap_err();
        assert!(
            err.contains("users/alice"),
            "error message should hint at users/<name>: {err}"
        );
    }

    #[test]
    fn parse_assignee_principal_rejects_empty_with_hint() {
        let err = parse_assignee_principal("").unwrap_err();
        assert!(
            err.contains("users/<name>"),
            "error should explain the path form: {err}"
        );
    }

    #[test]
    fn parse_assignee_principal_accepts_users_path() {
        let p = parse_assignee_principal("users/alice").unwrap();
        assert_eq!(p, user_principal("alice"));
    }

    #[test]
    fn parse_assignee_principal_accepts_agents_path() {
        let p = parse_assignee_principal("agents/swe").unwrap();
        assert_eq!(p.to_string(), "agents/swe");
    }

    #[test]
    fn parse_issue_dependency_parses_type_and_id() {
        let dependency = parse_issue_dependency("child-of:i-abcd").unwrap();
        assert_eq!(dependency.dependency_type, IssueDependencyType::ChildOf);
        assert_eq!(dependency.issue_id, issue_id("i-abcd"));
    }

    #[tokio::test]
    async fn update_issue_modifies_requested_fields() {
        let server = MockServer::start();
        let client = hydra_client(&server);
        let target_issue_id = issue_id("i-9");
        let mut job_settings = SessionSettings::default();
        job_settings.repo_name = Some(sample_repo_name());
        job_settings.remote_url = Some("https://example.com/service.git".into());
        job_settings.image = Some("worker:123".into());
        job_settings.branch = Some("main".into());
        job_settings.max_retries = Some(5);
        let current_issue = api_issue_record(
            "i-9",
            IssueType::Task,
            "Initial issue",
            IssueStatus::Open,
            Some("owner-a"),
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                issue_id("i-1"),
            )],
            Vec::new(),
        );
        let updated_request = UpsertIssueRequest::new(
            Issue::new(
                IssueType::Bug,
                "Test Title".to_string(),
                "Updated issue description".into(),
                empty_user(),
                "New progress".into(),
                IssueStatus::Closed.into(),
                None,
                Some(user_principal("owner-b")),
                Some(job_settings.clone()),
                vec![IssueDependency::new(
                    IssueDependencyType::BlockedOn,
                    issue_id("i-2"),
                )],
                vec![patch_id("p-3")],
                false,
                None,
                None,
                None,
            ),
            None,
        );
        let get_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{target_issue_id}").as_str());
            then.status(200).json_body_obj(&current_issue);
        });
        let update_mock = server.mock(|when, then| {
            when.method(PUT)
                .path(format!("/v1/issues/{target_issue_id}").as_str())
                .json_body_obj(&updated_request);
            then.status(200)
                .json_body_obj(&UpsertIssueResponse::new(target_issue_id.clone(), 0));
        });

        update_issue(
            &client,
            target_issue_id,
            Some(IssueType::Bug),
            None,
            Some(IssueStatus::Closed.into()),
            ProjectUpdate::Unchanged,
            Some(user_principal("owner-b")),
            false,
            Some("Updated issue description".into()),
            vec![IssueDependency::new(
                IssueDependencyType::BlockedOn,
                issue_id("i-2"),
            )],
            false,
            vec![patch_id("p-3")],
            false,
            Some("New progress".into()),
            false,
            Some("dourolabs/example".into()),
            Some("https://example.com/service.git".into()),
            Some("worker:123".into()),
            None,
            Some("main".into()),
            Some(5),
            Vec::new(),
            false,
            false,
            Vec::new(),
            Vec::new(),
            None,
            false,
            None,
            false,
        )
        .await
        .unwrap();

        get_mock.assert();
        update_mock.assert();
        assert_eq!(get_mock.hits(), 1);
        assert_eq!(update_mock.hits(), 1);
    }

    #[tokio::test]
    async fn update_issue_allows_clearing_assignee_and_dependencies() {
        let server = MockServer::start();
        let client = hydra_client(&server);
        let target_issue_id = issue_id("i-10");
        let current_issue = IssueVersionRecord::new(
            target_issue_id.clone(),
            0,
            Utc::now(),
            Issue::new(
                IssueType::Feature,
                "Test Title".to_string(),
                "Existing issue".into(),
                empty_user(),
                "Started work".into(),
                IssueStatus::InProgress.into(),
                None,
                Some(user_principal("owner-a")),
                None,
                vec![IssueDependency::new(
                    IssueDependencyType::BlockedOn,
                    issue_id("i-5"),
                )],
                Vec::new(),
                false,
                None,
                None,
                None,
            ),
            None,
            Utc::now(),
            Vec::new(),
        );
        let update_request = UpsertIssueRequest::new(
            Issue::new(
                IssueType::Feature,
                "Test Title".to_string(),
                "Existing issue".into(),
                empty_user(),
                String::new(),
                IssueStatus::InProgress.into(),
                None,
                None,
                None,
                vec![],
                Vec::new(),
                false,
                None,
                None,
                None,
            ),
            None,
        );
        let get_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{target_issue_id}").as_str());
            then.status(200).json_body_obj(&current_issue);
        });
        let update_mock = server.mock(|when, then| {
            when.method(PUT)
                .path(format!("/v1/issues/{target_issue_id}").as_str())
                .json_body_obj(&update_request);
            then.status(200)
                .json_body_obj(&UpsertIssueResponse::new(target_issue_id.clone(), 0));
        });

        update_issue(
            &client,
            target_issue_id,
            None,
            None,
            None,
            ProjectUpdate::Unchanged,
            None,
            true,
            None,
            vec![],
            true,
            vec![],
            true,
            None,
            true,
            None,
            None,
            None,
            None,
            None,
            None,
            Vec::new(),
            false,
            false,
            Vec::new(),
            Vec::new(),
            None,
            false,
            None,
            false,
        )
        .await
        .unwrap();

        get_mock.assert();
        update_mock.assert();
        assert_eq!(get_mock.hits(), 1);
        assert_eq!(update_mock.hits(), 1);
    }

    #[tokio::test]
    async fn update_issue_allows_clearing_job_settings() {
        let server = MockServer::start();
        let client = hydra_client(&server);
        let target_issue_id = issue_id("i-11");
        let job_settings = sample_job_settings();
        let current_issue = IssueVersionRecord::new(
            target_issue_id.clone(),
            0,
            Utc::now(),
            Issue::new(
                IssueType::Feature,
                "Test Title".to_string(),
                "Existing issue".into(),
                empty_user(),
                "Started work".into(),
                IssueStatus::InProgress.into(),
                None,
                Some(user_principal("owner-a")),
                Some(job_settings),
                vec![IssueDependency::new(
                    IssueDependencyType::BlockedOn,
                    issue_id("i-5"),
                )],
                Vec::new(),
                false,
                None,
                None,
                None,
            ),
            None,
            Utc::now(),
            Vec::new(),
        );
        let update_request = UpsertIssueRequest::new(
            Issue::new(
                IssueType::Feature,
                "Test Title".to_string(),
                "Existing issue".into(),
                empty_user(),
                "Started work".into(),
                IssueStatus::InProgress.into(),
                None,
                Some(user_principal("owner-a")),
                None,
                vec![IssueDependency::new(
                    IssueDependencyType::BlockedOn,
                    issue_id("i-5"),
                )],
                Vec::new(),
                false,
                None,
                None,
                None,
            ),
            None,
        );
        let get_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{target_issue_id}").as_str());
            then.status(200).json_body_obj(&current_issue);
        });
        let update_mock = server.mock(|when, then| {
            when.method(PUT)
                .path(format!("/v1/issues/{target_issue_id}").as_str())
                .json_body_obj(&update_request);
            then.status(200)
                .json_body_obj(&UpsertIssueResponse::new(target_issue_id.clone(), 0));
        });

        update_issue(
            &client,
            target_issue_id,
            None,
            None,
            None,
            ProjectUpdate::Unchanged,
            None,
            false,
            None,
            vec![],
            false,
            vec![],
            false,
            None,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            Vec::new(),
            false,
            true,
            Vec::new(),
            Vec::new(),
            None,
            false,
            None,
            false,
        )
        .await
        .unwrap();

        get_mock.assert();
        update_mock.assert();
        assert_eq!(get_mock.hits(), 1);
        assert_eq!(update_mock.hits(), 1);
    }

    #[tokio::test]
    async fn update_issue_sets_secrets() {
        let server = MockServer::start();
        let client = hydra_client(&server);
        let target_issue_id = issue_id("i-secrets");
        let current_issue = IssueVersionRecord::new(
            target_issue_id.clone(),
            0,
            Utc::now(),
            Issue::new(
                IssueType::Task,
                "Test Title".to_string(),
                "Existing issue".into(),
                empty_user(),
                String::new(),
                IssueStatus::Open.into(),
                None,
                None,
                None,
                Vec::new(),
                Vec::new(),
                false,
                None,
                None,
                None,
            ),
            None,
            Utc::now(),
            Vec::new(),
        );
        let mut expected_settings = SessionSettings::default();
        expected_settings.secrets = Some(vec!["new-secret".into()]);
        let update_request = UpsertIssueRequest::new(
            Issue::new(
                IssueType::Task,
                "Test Title".to_string(),
                "Existing issue".into(),
                empty_user(),
                String::new(),
                IssueStatus::Open.into(),
                None,
                None,
                Some(expected_settings),
                Vec::new(),
                Vec::new(),
                false,
                None,
                None,
                None,
            ),
            None,
        );
        let get_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{target_issue_id}").as_str());
            then.status(200).json_body_obj(&current_issue);
        });
        let update_mock = server.mock(|when, then| {
            when.method(PUT)
                .path(format!("/v1/issues/{target_issue_id}").as_str())
                .json_body_obj(&update_request);
            then.status(200)
                .json_body_obj(&UpsertIssueResponse::new(target_issue_id.clone(), 0));
        });

        update_issue(
            &client,
            target_issue_id,
            None,
            None,
            None,
            ProjectUpdate::Unchanged,
            None,
            false,
            None,
            vec![],
            false,
            vec![],
            false,
            None,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            vec!["new-secret".into()],
            false,
            false,
            Vec::new(),
            Vec::new(),
            None,
            false,
            None,
            false,
        )
        .await
        .unwrap();

        get_mock.assert();
        update_mock.assert();
        assert_eq!(get_mock.hits(), 1);
        assert_eq!(update_mock.hits(), 1);
    }

    #[tokio::test]
    async fn update_issue_allows_clearing_secrets() {
        let server = MockServer::start();
        let client = hydra_client(&server);
        let target_issue_id = issue_id("i-clear-secrets");
        let mut existing_settings = SessionSettings::default();
        existing_settings.secrets = Some(vec!["old-secret".into()]);
        let current_issue = IssueVersionRecord::new(
            target_issue_id.clone(),
            0,
            Utc::now(),
            Issue::new(
                IssueType::Task,
                "Test Title".to_string(),
                "Existing issue".into(),
                empty_user(),
                String::new(),
                IssueStatus::Open.into(),
                None,
                None,
                Some(existing_settings),
                Vec::new(),
                Vec::new(),
                false,
                None,
                None,
                None,
            ),
            None,
            Utc::now(),
            Vec::new(),
        );
        let update_request = UpsertIssueRequest::new(
            Issue::new(
                IssueType::Task,
                "Test Title".to_string(),
                "Existing issue".into(),
                empty_user(),
                String::new(),
                IssueStatus::Open.into(),
                None,
                None,
                Some(SessionSettings::default()),
                Vec::new(),
                Vec::new(),
                false,
                None,
                None,
                None,
            ),
            None,
        );
        let get_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{target_issue_id}").as_str());
            then.status(200).json_body_obj(&current_issue);
        });
        let update_mock = server.mock(|when, then| {
            when.method(PUT)
                .path(format!("/v1/issues/{target_issue_id}").as_str())
                .json_body_obj(&update_request);
            then.status(200)
                .json_body_obj(&UpsertIssueResponse::new(target_issue_id.clone(), 0));
        });

        update_issue(
            &client,
            target_issue_id,
            None,
            None,
            None,
            ProjectUpdate::Unchanged,
            None,
            false,
            None,
            vec![],
            false,
            vec![],
            false,
            None,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            Vec::new(),
            true,
            false,
            Vec::new(),
            Vec::new(),
            None,
            false,
            None,
            false,
        )
        .await
        .unwrap();

        get_mock.assert();
        update_mock.assert();
        assert_eq!(get_mock.hits(), 1);
        assert_eq!(update_mock.hits(), 1);
    }

    #[test]
    fn pretty_prints_human_readable_issues() {
        let issues = vec![
            IssueVersionRecord::new(
                issue_id("i-1"),
                0,
                Utc::now(),
                Issue::new(
                    IssueType::Bug,
                    "First Title".to_string(),
                    "First issue\nwith context".into(),
                    empty_user(),
                    "Working on repro".into(),
                    IssueStatus::Open.into(),
                    None,
                    Some(user_principal("owner-a")),
                    None,
                    vec![IssueDependency::new(
                        IssueDependencyType::BlockedOn,
                        issue_id("i-99"),
                    )],
                    Vec::new(),
                    false,
                    None,
                    None,
                    None,
                ),
                None,
                Utc::now(),
                Vec::new(),
            ),
            IssueVersionRecord::new(
                issue_id("i-2"),
                0,
                Utc::now(),
                Issue::new(
                    IssueType::Feature,
                    "Second Title".to_string(),
                    "Follow-up work".into(),
                    empty_user(),
                    String::new(),
                    IssueStatus::InProgress.into(),
                    None,
                    None,
                    None,
                    vec![],
                    Vec::new(),
                    false,
                    None,
                    None,
                    None,
                ),
                None,
                Utc::now(),
                Vec::new(),
            ),
        ];

        let mut output = Vec::new();
        render(
            IssueRecords(&issues),
            ResolvedOutputFormat::Pretty,
            &mut output,
        )
        .unwrap();
        let rendered = String::from_utf8(output).unwrap();
        let first_issue = issue_id("i-1").to_string();
        let dependency_id = issue_id("i-99").to_string();
        let second_issue = issue_id("i-2").to_string();

        assert!(rendered.contains(&format!("Issue {first_issue} (bug, open)")));
        assert!(rendered.contains("Assignee: users/owner-a"));
        assert!(rendered.contains("Description:\n  First issue\n  with context"));
        assert!(rendered.contains("Progress:\n  Working on repro"));
        assert!(rendered.contains(&format!("Dependencies:\n  - blocked-on {dependency_id}")));
        assert!(rendered.contains(&format!("Issue {second_issue} (feature, in-progress)")));
        assert!(rendered.contains("Assignee: -"));
        assert!(rendered.contains("Progress:\n  -"));
        assert!(rendered.contains("Dependencies: none"));
        assert!(rendered.contains("Follow-up work"));
    }

    // ---- Label helper tests ----

    use crate::test_utils::ids::label_id;
    use hydra_common::api::v1::labels::{LabelRecord, ListLabelsResponse, UpsertLabelResponse};
    use hydra_common::rgb::Rgb;

    fn sample_label_record(id: &str, name: &str, color: &str) -> LabelRecord {
        LabelRecord::new(
            label_id(id),
            name.to_string(),
            color.parse::<Rgb>().unwrap(),
            true,
            false,
            Utc::now(),
            Utc::now(),
        )
    }

    #[tokio::test]
    async fn resolve_label_names_to_ids_happy_path() {
        let server = MockServer::start();
        let client = hydra_client(&server);
        let frontend_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/labels")
                .query_param("q", "frontend");
            then.status(200)
                .json_body_obj(&ListLabelsResponse::new(vec![sample_label_record(
                    "l-aaaa", "frontend", "#e74c3c",
                )]));
        });
        let urgent_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/labels")
                .query_param("q", "urgent");
            then.status(200)
                .json_body_obj(&ListLabelsResponse::new(vec![sample_label_record(
                    "l-bbbb", "urgent", "#3498db",
                )]));
        });

        let ids =
            resolve_label_names_to_ids(&client, &["frontend".to_string(), "urgent".to_string()])
                .await
                .unwrap();

        frontend_mock.assert();
        urgent_mock.assert();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], label_id("l-aaaa"));
        assert_eq!(ids[1], label_id("l-bbbb"));
    }

    #[tokio::test]
    async fn resolve_label_names_to_ids_case_insensitive() {
        let server = MockServer::start();
        let client = hydra_client(&server);
        let list_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/labels")
                .query_param("q", "FRONTEND");
            then.status(200)
                .json_body_obj(&ListLabelsResponse::new(vec![sample_label_record(
                    "l-aaaa", "Frontend", "#e74c3c",
                )]));
        });

        let ids = resolve_label_names_to_ids(&client, &["FRONTEND".to_string()])
            .await
            .unwrap();

        list_mock.assert();
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], label_id("l-aaaa"));
    }

    #[tokio::test]
    async fn resolve_label_names_to_ids_missing_label_errors() {
        let server = MockServer::start();
        let client = hydra_client(&server);
        server.mock(|when, then| {
            when.method(GET)
                .path("/v1/labels")
                .query_param("q", "nonexistent");
            then.status(200)
                .json_body_obj(&ListLabelsResponse::new(vec![]));
        });

        let result = resolve_label_names_to_ids(&client, &["nonexistent".to_string()]).await;

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("nonexistent"),
            "error should mention the missing label name, got: {err_msg}"
        );
    }

    #[tokio::test]
    async fn resolve_label_names_to_ids_empty_input() {
        let server = MockServer::start();
        let client = hydra_client(&server);

        let ids = resolve_label_names_to_ids(&client, &[]).await.unwrap();
        assert!(ids.is_empty());
    }

    #[tokio::test]
    async fn resolve_or_create_label_finds_existing() {
        let server = MockServer::start();
        let client = hydra_client(&server);
        let existing = sample_label_record("l-aaaa", "frontend", "#e74c3c");

        let id = resolve_or_create_label(&client, "Frontend", &[existing])
            .await
            .unwrap();
        assert_eq!(id, label_id("l-aaaa"));
    }

    #[tokio::test]
    async fn resolve_or_create_label_creates_when_not_found() {
        let server = MockServer::start();
        let client = hydra_client(&server);
        let new_label_id = label_id("l-cccc");
        let create_response = UpsertLabelResponse::new(new_label_id.clone());
        let create_mock = server.mock(|when, then| {
            when.method(POST).path("/v1/labels");
            then.status(200).json_body_obj(&create_response);
        });

        let id = resolve_or_create_label(&client, "new-label", &[])
            .await
            .unwrap();

        create_mock.assert();
        assert_eq!(id, label_id("l-cccc"));
    }

    #[tokio::test]
    async fn apply_label_changes_adds_and_removes() {
        let server = MockServer::start();
        let client = hydra_client(&server);
        let existing_labels = vec![
            sample_label_record("l-aaaa", "frontend", "#e74c3c"),
            sample_label_record("l-bbbb", "urgent", "#3498db"),
        ];
        let labels_response = ListLabelsResponse::new(existing_labels);

        let list_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/labels");
            then.status(200).json_body_obj(&labels_response);
        });

        let object_id: HydraId = issue_id("i-target").into();

        // Mock the add association endpoint for "frontend"
        let add_mock = server.mock(|when, then| {
            when.method(PUT)
                .path_matches(httpmock::Regex::new(r"/v1/labels/.*/objects/.*").unwrap());
            then.status(200);
        });

        // Mock the remove association endpoint for "urgent"
        let remove_mock = server.mock(|when, then| {
            when.method(DELETE)
                .path_matches(httpmock::Regex::new(r"/v1/labels/.*/objects/.*").unwrap());
            then.status(200);
        });

        apply_label_changes(
            &client,
            &object_id,
            &["frontend".to_string()],
            &["urgent".to_string()],
        )
        .await
        .unwrap();

        list_mock.assert();
        add_mock.assert();
        remove_mock.assert();
    }

    #[tokio::test]
    async fn apply_label_changes_noop_when_empty() {
        let server = MockServer::start();
        let client = hydra_client(&server);
        let object_id: HydraId = issue_id("i-target").into();

        // No mocks needed — the function should return immediately
        apply_label_changes(&client, &object_id, &[], &[])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn apply_label_changes_remove_missing_label_errors() {
        let server = MockServer::start();
        let client = hydra_client(&server);
        let labels_response =
            ListLabelsResponse::new(vec![sample_label_record("l-aaaa", "frontend", "#e74c3c")]);
        server.mock(|when, then| {
            when.method(GET).path("/v1/labels");
            then.status(200).json_body_obj(&labels_response);
        });

        let object_id: HydraId = issue_id("i-target").into();
        let result =
            apply_label_changes(&client, &object_id, &[], &["nonexistent".to_string()]).await;

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("nonexistent"),
            "error should mention the missing label name, got: {err_msg}"
        );
    }

    #[tokio::test]
    async fn fetch_issues_passes_label_ids() {
        let server = MockServer::start();
        let client = hydra_client(&server);
        let lid = label_id("l-aaaa");
        let issues_response = ListIssuesResponse::new(vec![]);
        let list_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/issues")
                .query_param("labels", lid.to_string());
            then.status(200).json_body_obj(&issues_response);
        });

        let issues = fetch_issues(
            &client,
            None,
            None,
            Vec::new(),
            None,
            None,
            None,
            vec![lid.clone()],
            false,
        )
        .await
        .unwrap();

        list_mock.assert();
        assert!(issues.is_empty());
    }

    #[tokio::test]
    async fn fetch_issues_passes_status_key_filter() {
        // CLI passes `--status inbox,triage` straight through to the
        // wire `?status=inbox,triage` query — confirms that per-project
        // status keys reach the server intact (previously they were
        // silently coerced to `IssueStatus::Unknown` and dropped).
        let server = MockServer::start();
        let client = hydra_client(&server);
        let issues_response = ListIssuesResponse::new(vec![]);
        let list_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/issues")
                .query_param("status", "inbox,triage");
            then.status(200).json_body_obj(&issues_response);
        });

        let issues = fetch_issues(
            &client,
            None,
            None,
            vec![
                StatusKey::try_new("inbox").unwrap(),
                StatusKey::try_new("triage").unwrap(),
            ],
            None,
            None,
            None,
            Vec::new(),
            false,
        )
        .await
        .unwrap();

        list_mock.assert();
        assert!(issues.is_empty());
    }

    #[tokio::test]
    async fn fetch_issues_passes_project_id_filter() {
        let server = MockServer::start();
        let client = hydra_client(&server);
        let issues_response = ListIssuesResponse::new(vec![]);
        let project_id = ProjectId::new();
        let list_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/issues")
                .query_param("project_id", project_id.as_ref());
            then.status(200).json_body_obj(&issues_response);
        });

        let issues = fetch_issues(
            &client,
            None,
            None,
            Vec::new(),
            Some(project_id),
            None,
            None,
            Vec::new(),
            false,
        )
        .await
        .unwrap();

        list_mock.assert();
        assert!(issues.is_empty());
    }

    #[test]
    fn list_command_parses_multi_status_keys_and_project_key() {
        // `hydra issues list --status inbox,in-progress --project engineering-v2`
        // must parse cleanly: status values are comma-split into StatusKey
        // entries, and the project token is resolved as a ProjectKey (since
        // it's not a valid ProjectId).
        use crate::cli::Cli;
        use clap::Parser;
        let cli = Cli::try_parse_from([
            "hydra",
            "issues",
            "list",
            "--status",
            "inbox,in-progress",
            "--project",
            "engineering-v2",
        ])
        .expect("CLI should parse");
        match cli.command.expect("subcommand must be present") {
            crate::cli::Commands::Issues { command } => match command {
                IssueCommands::List {
                    status, project, ..
                } => {
                    assert_eq!(
                        status,
                        vec![
                            StatusKey::try_new("inbox").unwrap(),
                            StatusKey::try_new("in-progress").unwrap(),
                        ]
                    );
                    let project = project.expect("project flag should parse");
                    match project {
                        crate::command::projects::ProjectRef::Key(key) => {
                            assert_eq!(key.as_str(), "engineering-v2");
                        }
                        other => panic!("expected ProjectRef::Key, got {other:?}"),
                    }
                }
                _ => panic!("expected IssueCommands::List"),
            },
            _ => panic!("expected Commands::Issues"),
        }
    }

    #[test]
    fn list_command_rejects_invalid_status_key() {
        // Uppercase characters and spaces are not valid StatusKey input;
        // the wire deserialize side enforces lowercase ASCII + digits + `-`,
        // and the CLI matches.
        use crate::cli::Cli;
        use clap::Parser;
        let result = Cli::try_parse_from(["hydra", "issues", "list", "--status", "Bad Status"]);
        let err = match result {
            Ok(_) => panic!("malformed status key should be rejected"),
            Err(err) => err,
        };
        let msg = err.to_string();
        assert!(
            msg.contains("--status"),
            "expected --status in error: {msg}"
        );
    }

    fn sample_submit_form_response() -> hydra_common::issues::SubmitFormResponse {
        use hydra_common::actor_ref::ActorId;
        use hydra_common::api::v1::form::FormResponse;

        let response = FormResponse {
            action_id: "approve".to_string(),
            actor: ActorId::User(hydra_common::api::v1::users::Username::try_new("alice").unwrap()),
            values: std::collections::HashMap::new(),
            submitted_at: Utc::now(),
        };
        hydra_common::issues::SubmitFormResponse::new(issue_id("i-42"), 7, response)
    }

    #[test]
    fn submit_form_outcome_renders_pretty_line() {
        let response = sample_submit_form_response();
        let mut buf = Vec::new();
        render(
            SubmitFormOutcome(&response),
            ResolvedOutputFormat::Pretty,
            &mut buf,
        )
        .unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert_eq!(
            output,
            format!(
                "Submitted form for issue '{}' (action: 'approve', version: 7)\n",
                issue_id("i-42")
            )
        );
    }

    #[test]
    fn submit_form_outcome_renders_jsonl_object() {
        let response = sample_submit_form_response();
        let mut buf = Vec::new();
        render(
            SubmitFormOutcome(&response),
            ResolvedOutputFormat::Jsonl,
            &mut buf,
        )
        .unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.ends_with('\n'), "jsonl output must end with newline");
        let trimmed = output.trim_end_matches('\n');
        assert!(
            !trimmed.contains('\n'),
            "jsonl output must be exactly one line"
        );
        let parsed: serde_json::Value = serde_json::from_str(trimmed).unwrap();
        assert_eq!(parsed["issue_id"], issue_id("i-42").to_string());
        assert_eq!(parsed["version"], 7);
        assert_eq!(parsed["form_response"]["action_id"], "approve");
    }
}
