use crate::{
    client::MetisClientInterface,
    command::output::{render_issue_records, CommandContext, ResolvedOutputFormat},
};
use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use clap::Subcommand;
use metis_common::{
    activity_log_for_issue_versions, activity_log_for_job_versions,
    activity_log_for_patch_versions,
    constants::ENV_METIS_ISSUE_ID,
    issues::{
        AddTodoItemRequest, Issue, IssueDependency, IssueDependencyType, IssueGraphFilter,
        IssueGraphSelector, IssueGraphWildcard, IssueId, IssueStatus, IssueType,
        IssueVersionRecord, JobSettings, ReplaceTodoListRequest, SearchIssuesQuery,
        SetTodoItemStatusRequest, TodoItem, UpsertIssueRequest,
    },
    jobs::{JobVersionRecord, SearchJobsQuery, Task},
    patches::{PatchStatus, PatchVersionRecord, Review},
    task_status::Status,
    users::Username,
    whoami::ActorIdentity,
    ActivityEvent, ActivityLogEntry, ActivityObjectKind, FieldChange, MetisId, PatchId, RepoName,
    TaskId, VersionNumber, Versioned,
};
use owo_colors::OwoColorize;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    io::{self, Write},
    str::FromStr,
};

#[derive(Debug, Subcommand)]
pub enum IssueCommands {
    /// List Metis issues.
    List {
        /// Filter by issue ID.
        #[arg(long, value_name = "ISSUE_ID", conflicts_with = "query")]
        id: Option<IssueId>,

        /// Filter by issue type.
        #[arg(long, value_name = "ISSUE_TYPE")]
        r#type: Option<IssueType>,

        /// Filter by issue status.
        #[arg(long, value_name = "ISSUE_STATUS")]
        status: Option<IssueStatus>,

        /// Filter by assignee.
        #[arg(long, value_name = "ASSIGNEE")]
        assignee: Option<String>,

        /// Search by query string.
        #[arg(long, value_name = "QUERY")]
        query: Option<String>,

        /// Filter by dependency graph relationships (e.g. '*:child-of:i-abc' or '**:blocked-on:i-def').
        #[arg(
            long = "graph",
            value_name = "FILTER",
            value_parser = parse_issue_graph_filter,
            conflicts_with = "id"
        )]
        graph_filters: Vec<IssueGraphFilter>,

        /// Include deleted issues in the listing.
        #[arg(long = "include-deleted")]
        include_deleted: bool,
    },
    /// Create a new issue.
    Create {
        /// Issue type: bug, feature, task, chore, or merge-request (defaults to task).
        #[arg(long, value_name = "ISSUE_TYPE", default_value_t = IssueType::Task)]
        r#type: IssueType,

        /// Issue status: open, in-progress, or closed (defaults to open).
        #[arg(long, value_name = "ISSUE_STATUS", default_value_t = IssueStatus::Open)]
        status: IssueStatus,

        /// Issue dependencies in the format dependency-type:ISSUE_ID where dependency-type is child-of or blocked-on (e.g. child-of:i-abcd).
        #[arg(long = "deps", value_name = "TYPE:ISSUE_ID", value_parser = parse_issue_dependency)]
        dependencies: Vec<IssueDependency>,

        /// Patch ids to associate with the issue.
        #[arg(long = "patches", value_name = "PATCH_ID", value_delimiter = ',')]
        patches: Vec<PatchId>,

        /// Assignee for the issue.
        #[arg(long, value_name = "ASSIGNEE")]
        assignee: Option<String>,

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
            env = ENV_METIS_ISSUE_ID
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

        /// Kubernetes secrets to use for job settings (comma-separated).
        #[arg(long, value_name = "SECRETS", value_delimiter = ',')]
        secrets: Vec<String>,
    },
    /// Update an existing issue.
    Update {
        /// Issue ID to update.
        #[arg(value_name = "ISSUE_ID")]
        id: IssueId,

        /// New issue type.
        #[arg(long, value_name = "ISSUE_TYPE")]
        r#type: Option<IssueType>,

        /// New issue status.
        #[arg(long, value_name = "ISSUE_STATUS")]
        status: Option<IssueStatus>,

        /// Updated assignee.
        #[arg(long, value_name = "ASSIGNEE", conflicts_with = "clear_assignee")]
        assignee: Option<String>,

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

        /// Kubernetes secrets to use for job settings (comma-separated).
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
    },
    /// Inspect or update an issue's todo list.
    Todo {
        /// Issue ID to operate on.
        #[arg(value_name = "ISSUE_ID")]
        id: IssueId,

        /// Append a new todo item (prefix with '[x]' to mark done immediately).
        #[arg(long, value_name = "TEXT", conflicts_with_all = ["done", "undone", "replace"])]
        add: Option<String>,

        /// Mark a todo item as done (1-indexed).
        #[arg(
            long,
            value_name = "ITEM_NUMBER",
            value_parser = clap::value_parser!(usize),
            conflicts_with_all = ["add", "undone", "replace"]
        )]
        done: Option<usize>,

        /// Mark a todo item as not done (1-indexed).
        #[arg(
            long,
            value_name = "ITEM_NUMBER",
            value_parser = clap::value_parser!(usize),
            conflicts_with_all = ["add", "done", "replace"]
        )]
        undone: Option<usize>,

        /// Replace the entire todo list with the provided ordered items.
        #[arg(
            long,
            value_name = "ITEM",
            num_args = 1..,
            value_delimiter = ',',
            conflicts_with_all = ["add", "done", "undone"]
        )]
        replace: Option<Vec<String>>,
    },
    /// Describe an issue and its relationships.
    Describe {
        /// Issue ID to describe.
        #[arg(value_name = "ISSUE_ID")]
        id: IssueId,

        /// Emit the complete JSONL output instead of the summarized view.
        #[arg(long)]
        verbose: bool,
    },
    /// Delete an issue.
    Delete {
        /// Issue ID to delete.
        #[arg(value_name = "ISSUE_ID")]
        id: IssueId,
    },
}

pub async fn run(
    client: &dyn MetisClientInterface,
    command: IssueCommands,
    context: &CommandContext,
) -> Result<()> {
    match command {
        IssueCommands::List {
            id,
            r#type,
            status,
            assignee,
            query,
            graph_filters,
            include_deleted,
        } => {
            let issues = fetch_issues(
                client,
                id,
                r#type,
                status,
                assignee,
                query,
                graph_filters,
                include_deleted,
            )
            .await?;
            write_issue_records(context.output_format, &issues)?;
            Ok(())
        }
        IssueCommands::Create {
            r#type,
            status,
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
        } => {
            let creator =
                resolve_creator_username(client, &dependencies, current_issue_id.as_ref()).await?;
            create_issue(
                client,
                r#type,
                status,
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
            )
            .await
            .and_then(|issue| write_issue_records(context.output_format, &[issue]))
        }
        IssueCommands::Update {
            id,
            r#type,
            status,
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
        } => update_issue(
            client,
            id,
            r#type,
            status,
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
        )
        .await
        .and_then(|issue| write_issue_records(context.output_format, &[issue])),
        IssueCommands::Todo {
            id,
            add,
            done,
            undone,
            replace,
        } => {
            manage_todo_list(
                client,
                id,
                add,
                done,
                undone,
                replace,
                context.output_format,
            )
            .await
        }
        IssueCommands::Describe { id, verbose } => {
            describe_issue(client, id, context.output_format, verbose).await
        }
        IssueCommands::Delete { id } => {
            let deleted = client
                .delete_issue(&id)
                .await
                .with_context(|| format!("failed to delete issue '{id}'"))?;
            println!("Deleted issue '{}'", deleted.issue_id);
            Ok(())
        }
    }
}

#[derive(Debug, Serialize, PartialEq, Eq, Clone)]
struct IssueWithPatches {
    issue: IssueVersionRecord,
    patches: Vec<PatchVersionRecord>,
}

#[derive(Debug, Serialize)]
struct IssueDescription {
    issue: IssueWithPatches,
    parents: Vec<IssueWithPatches>,
    children: Vec<IssueWithPatches>,
    activity_log: Vec<ActivityLogEntry>,
}

#[derive(Debug, Serialize)]
struct IssueDescriptionSummary {
    issue: IssueWithPatches,
    parents: Vec<IssueId>,
    children: Vec<IssueId>,
    activity_log: Vec<ActivityLogEntrySummary>,
}

#[derive(Debug, Serialize)]
struct ActivityLogEntrySummary {
    object_id: MetisId,
    object_kind: ActivityObjectKind,
    version: VersionNumber,
    timestamp: DateTime<Utc>,
    event: ActivityEventSummary,
    object: ActivityObjectSummary,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ActivityEventSummary {
    Created,
    Updated {
        changes: Vec<ActivityFieldChangeSummary>,
        #[serde(skip_serializing_if = "Vec::is_empty", default)]
        other_changes: Vec<String>,
    },
}

#[derive(Debug, Serialize)]
struct ActivityFieldChangeSummary {
    field: String,
    before: Value,
    after: Value,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ActivityObjectSummary {
    Issue {
        issue_type: IssueType,
        status: IssueStatus,
        description: String,
        assignee: Option<String>,
        progress: String,
    },
    Patch {
        title: String,
        description: String,
        status: PatchStatus,
        repo: RepoName,
        created_by_job: Option<TaskId>,
        reviews: Vec<ReviewSummary>,
    },
    Job {
        status: Status,
    },
}

#[derive(Debug, Serialize)]
struct ReviewSummary {
    contents: String,
    is_approved: bool,
    author: String,
}

#[derive(Debug, Serialize)]
struct TodoListOutput<'a> {
    issue_id: &'a IssueId,
    todo_list: &'a [TodoItem],
}

async fn describe_issue(
    client: &dyn MetisClientInterface,
    id: IssueId,
    output_format: ResolvedOutputFormat,
    verbose: bool,
) -> Result<()> {
    let description = collect_issue_description(client, id).await?;
    let summary = summarize_issue_description(&description)?;

    let mut buffer = Vec::new();
    if verbose {
        serde_json::to_writer(&mut buffer, &description)?;
        buffer.write_all(b"\n")?;
    } else {
        match output_format {
            ResolvedOutputFormat::Pretty => {
                print_issue_description_pretty(&summary, &mut buffer)?;
            }
            ResolvedOutputFormat::Jsonl => {
                serde_json::to_writer(&mut buffer, &summary)?;
                buffer.write_all(b"\n")?;
            }
        }
    }
    io::stdout().write_all(&buffer)?;
    io::stdout().flush()?;

    Ok(())
}

async fn collect_issue_description(
    client: &dyn MetisClientInterface,
    issue_id: IssueId,
) -> Result<IssueDescription> {
    let issue = client
        .get_issue(&issue_id)
        .await
        .with_context(|| format!("failed to fetch issue '{issue_id}'"))?;

    let parents = fetch_parent_issues(client, &issue).await?;
    let children = fetch_child_issues(client, &issue.issue_id).await?;
    let mut patch_cache = HashMap::new();

    let issue_with_patches = issue_with_patches(client, issue, &mut patch_cache).await?;
    let parents_with_patches = issues_with_patches(client, parents, &mut patch_cache).await?;
    let children_with_patches = issues_with_patches(client, children, &mut patch_cache).await?;
    let activity_log = collect_activity_log(
        client,
        &issue_with_patches,
        &parents_with_patches,
        &children_with_patches,
    )
    .await?;

    Ok(IssueDescription {
        issue: issue_with_patches,
        parents: parents_with_patches,
        children: children_with_patches,
        activity_log,
    })
}

fn summarize_issue_description(description: &IssueDescription) -> Result<IssueDescriptionSummary> {
    Ok(IssueDescriptionSummary {
        issue: description.issue.clone(),
        parents: description
            .parents
            .iter()
            .map(|parent| parent.issue.issue_id.clone())
            .collect(),
        children: description
            .children
            .iter()
            .map(|child| child.issue.issue_id.clone())
            .collect(),
        activity_log: summarize_activity_log(&description.activity_log)?,
    })
}

fn summarize_activity_log(entries: &[ActivityLogEntry]) -> Result<Vec<ActivityLogEntrySummary>> {
    entries.iter().map(summarize_activity_log_entry).collect()
}

fn summarize_activity_log_entry(entry: &ActivityLogEntry) -> Result<ActivityLogEntrySummary> {
    let object = summarize_activity_object(entry)?;
    let event = summarize_activity_event(entry, &object)?;

    Ok(ActivityLogEntrySummary {
        object_id: entry.object_id.clone(),
        object_kind: entry.object_kind.clone(),
        version: entry.version,
        timestamp: entry.timestamp,
        event,
        object,
    })
}

fn summarize_activity_object(entry: &ActivityLogEntry) -> Result<ActivityObjectSummary> {
    match entry.object_kind {
        ActivityObjectKind::Issue => {
            let issue: Issue = decode_activity_object(entry)?;
            Ok(ActivityObjectSummary::Issue {
                issue_type: issue.issue_type,
                status: issue.status,
                description: issue.description,
                assignee: issue.assignee,
                progress: issue.progress,
            })
        }
        ActivityObjectKind::Patch => {
            let patch: metis_common::patches::Patch = decode_activity_object(entry)?;
            Ok(ActivityObjectSummary::Patch {
                title: if patch.title.trim().is_empty() {
                    "(untitled)".to_string()
                } else {
                    patch.title
                },
                description: patch.description,
                status: patch.status,
                repo: patch.service_repo_name,
                created_by_job: patch.created_by,
                reviews: patch
                    .reviews
                    .into_iter()
                    .map(|review| ReviewSummary {
                        contents: review.contents,
                        is_approved: review.is_approved,
                        author: review.author,
                    })
                    .collect(),
            })
        }
        ActivityObjectKind::Job => {
            let task: Task = decode_activity_object(entry)?;
            Ok(ActivityObjectSummary::Job {
                status: task.status,
            })
        }
        _ => Ok(ActivityObjectSummary::Job {
            status: Status::Unknown,
        }),
    }
}

fn summarize_activity_event(
    entry: &ActivityLogEntry,
    object: &ActivityObjectSummary,
) -> Result<ActivityEventSummary> {
    match &entry.event {
        ActivityEvent::Created => Ok(ActivityEventSummary::Created),
        ActivityEvent::Updated { changes } => {
            let (summaries, other_changes) = summarize_activity_changes(entry, changes, object)?;
            Ok(ActivityEventSummary::Updated {
                changes: summaries,
                other_changes,
            })
        }
        _ => Ok(ActivityEventSummary::Updated {
            changes: Vec::new(),
            other_changes: vec!["<unsupported event>".to_string()],
        }),
    }
}

fn summarize_activity_changes(
    entry: &ActivityLogEntry,
    changes: &[FieldChange],
    object: &ActivityObjectSummary,
) -> Result<(Vec<ActivityFieldChangeSummary>, Vec<String>)> {
    let mut summaries = Vec::new();
    let mut other_changes = Vec::new();
    let mut seen = HashSet::new();

    let mut before_patch: Option<metis_common::patches::Patch> = None;
    let mut after_patch: Option<metis_common::patches::Patch> = None;
    if matches!(object, ActivityObjectSummary::Patch { .. })
        && changes
            .iter()
            .any(|change| change.path.starts_with("/reviews"))
    {
        if let Some(before_value) = reconstruct_before_object(entry) {
            before_patch = serde_json::from_value(before_value).ok();
        }
        after_patch = serde_json::from_value(entry.object.clone()).ok();
    }

    for change in changes {
        if let Some(field) = tracked_field_for_path(&entry.object_kind, &change.path) {
            if seen.contains(field) {
                continue;
            }
            seen.insert(field);

            if field == "Reviews" {
                let before_value = summarize_reviews_value(
                    before_patch.as_ref().map(|patch| patch.reviews.as_slice()),
                );
                let after_value = summarize_reviews_value(
                    after_patch.as_ref().map(|patch| patch.reviews.as_slice()),
                );
                summaries.push(ActivityFieldChangeSummary {
                    field: field.to_string(),
                    before: before_value,
                    after: after_value,
                });
            } else {
                summaries.push(ActivityFieldChangeSummary {
                    field: field.to_string(),
                    before: change.before.clone(),
                    after: change.after.clone(),
                });
            }
        } else {
            other_changes.push(change.path.clone());
        }
    }

    Ok((summaries, other_changes))
}

fn tracked_field_for_path(kind: &ActivityObjectKind, path: &str) -> Option<&'static str> {
    match kind {
        ActivityObjectKind::Issue => match path {
            "/type" => Some("Type"),
            "/status" => Some("Status"),
            "/description" => Some("Description"),
            "/assignee" => Some("Assignee"),
            "/progress" => Some("Progress"),
            _ => None,
        },
        ActivityObjectKind::Patch => {
            if path.starts_with("/reviews") {
                Some("Reviews")
            } else {
                match path {
                    "/title" => Some("Title"),
                    "/description" => Some("Description"),
                    "/status" => Some("Status"),
                    "/service_repo_name" => Some("Repo"),
                    "/created_by" => Some("Created By Job"),
                    _ => None,
                }
            }
        }
        ActivityObjectKind::Job => match path {
            "/status" => Some("Status"),
            _ => None,
        },
        _ => None,
    }
}

fn summarize_reviews_value(reviews: Option<&[Review]>) -> Value {
    let summaries: Vec<ReviewSummary> = reviews
        .unwrap_or(&[])
        .iter()
        .map(|review| ReviewSummary {
            contents: review.contents.clone(),
            is_approved: review.is_approved,
            author: review.author.clone(),
        })
        .collect();
    serde_json::to_value(summaries).unwrap_or(Value::Null)
}

fn reconstruct_before_object(entry: &ActivityLogEntry) -> Option<Value> {
    let ActivityEvent::Updated { changes } = &entry.event else {
        return None;
    };

    let mut before = entry.object.clone();
    for change in changes {
        apply_change(&mut before, &change.path, change.before.clone());
    }
    Some(before)
}

fn apply_change(value: &mut Value, path: &str, new_value: Value) {
    if path == "/" {
        *value = new_value;
        return;
    }

    let mut current = value;
    let mut segments = path.trim_start_matches('/').split('/').peekable();

    while let Some(segment) = segments.next() {
        let is_last = segments.peek().is_none();
        match current {
            Value::Object(map) => {
                if is_last {
                    map.insert(segment.to_string(), new_value);
                    return;
                }
                current = map
                    .entry(segment)
                    .or_insert_with(|| Value::Object(Default::default()));
            }
            Value::Array(list) => {
                let Ok(index) = segment.parse::<usize>() else {
                    return;
                };
                if index >= list.len() {
                    list.resize_with(index + 1, || Value::Null);
                }
                if is_last {
                    list[index] = new_value;
                    return;
                }
                current = &mut list[index];
            }
            _ => return,
        }
    }
}

fn decode_activity_object<T: DeserializeOwned>(entry: &ActivityLogEntry) -> Result<T> {
    serde_json::from_value(entry.object.clone()).context("failed to decode activity log object")
}

async fn fetch_parent_issues(
    client: &dyn MetisClientInterface,
    issue: &IssueVersionRecord,
) -> Result<Vec<IssueVersionRecord>> {
    let mut parents = Vec::new();
    let mut seen = HashSet::new();

    for dependency in &issue.issue.dependencies {
        if dependency.dependency_type != IssueDependencyType::ChildOf {
            continue;
        }
        if !seen.insert(dependency.issue_id.clone()) {
            continue;
        }

        let parent = client
            .get_issue(&dependency.issue_id)
            .await
            .with_context(|| format!("failed to fetch parent issue '{}'", dependency.issue_id))?;
        parents.push(parent);
    }

    Ok(parents)
}

async fn fetch_child_issues(
    client: &dyn MetisClientInterface,
    issue_id: &IssueId,
) -> Result<Vec<IssueVersionRecord>> {
    let filter = IssueGraphFilter::new(
        IssueGraphSelector::Wildcard(IssueGraphWildcard::Transitive),
        IssueDependencyType::ChildOf,
        IssueGraphSelector::Issue(issue_id.clone()),
    )
    .map_err(|err| anyhow!(err))?;

    let response = client
        .list_issues(&SearchIssuesQuery::new(
            None,
            None,
            None,
            None,
            vec![filter],
            None,
        ))
        .await
        .with_context(|| format!("failed to fetch children for issue '{issue_id}'"))?;

    Ok(response.issues)
}

async fn issues_with_patches(
    client: &dyn MetisClientInterface,
    issues: Vec<IssueVersionRecord>,
    cache: &mut HashMap<PatchId, PatchVersionRecord>,
) -> Result<Vec<IssueWithPatches>> {
    let mut enriched = Vec::with_capacity(issues.len());
    for issue in issues {
        enriched.push(issue_with_patches(client, issue, cache).await?);
    }
    Ok(enriched)
}

async fn issue_with_patches(
    client: &dyn MetisClientInterface,
    issue: IssueVersionRecord,
    cache: &mut HashMap<PatchId, PatchVersionRecord>,
) -> Result<IssueWithPatches> {
    let patches = fetch_patch_records(client, &issue.issue.patches, cache, &issue.issue_id).await?;
    Ok(IssueWithPatches { issue, patches })
}

async fn fetch_patch_records(
    client: &dyn MetisClientInterface,
    patch_ids: &[PatchId],
    cache: &mut HashMap<PatchId, PatchVersionRecord>,
    issue_id: &IssueId,
) -> Result<Vec<PatchVersionRecord>> {
    let mut patches = Vec::with_capacity(patch_ids.len());
    for patch_id in patch_ids {
        if let Some(record) = cache.get(patch_id) {
            patches.push(record.clone());
            continue;
        }

        let record = client.get_patch(patch_id).await.with_context(|| {
            format!("failed to fetch patch '{patch_id}' for issue '{issue_id}'")
        })?;
        cache.insert(patch_id.clone(), record.clone());
        patches.push(record);
    }

    Ok(patches)
}

async fn collect_activity_log(
    client: &dyn MetisClientInterface,
    issue: &IssueWithPatches,
    parents: &[IssueWithPatches],
    children: &[IssueWithPatches],
) -> Result<Vec<ActivityLogEntry>> {
    let issue_id = &issue.issue.issue_id;
    let issue_versions = fetch_issue_versions(client, issue_id).await?;
    let root_created_at = issue_versions.iter().map(|version| version.timestamp).min();

    let mut entries = activity_log_for_issue_versions(issue_id.clone(), &issue_versions);

    for related in parents.iter().chain(children.iter()) {
        let related_id = &related.issue.issue_id;
        let versions = fetch_issue_versions(client, related_id).await?;
        let log = activity_log_for_issue_versions(related_id.clone(), &versions);
        entries.extend(filter_activity_entries(log, root_created_at));
    }

    let patch_ids = collect_patch_ids(issue, parents, children);
    for patch_id in patch_ids {
        let versions = fetch_patch_versions(client, &patch_id).await?;
        let log = activity_log_for_patch_versions(patch_id.clone(), &versions);
        entries.extend(filter_activity_entries(log, root_created_at));
    }

    let jobs = fetch_jobs_for_issue(client, issue_id).await?;
    for job in jobs {
        let versions = fetch_job_versions(client, &job.job_id).await?;
        let log = activity_log_for_job_versions(job.job_id.clone(), &versions);
        entries.extend(filter_activity_entries(log, root_created_at));
    }

    sort_activity_log_entries(&mut entries);
    Ok(entries)
}

fn collect_patch_ids(
    issue: &IssueWithPatches,
    parents: &[IssueWithPatches],
    children: &[IssueWithPatches],
) -> Vec<PatchId> {
    let mut ids = std::collections::BTreeSet::new();
    for issue in parents
        .iter()
        .chain(children.iter())
        .chain(std::iter::once(issue))
    {
        for patch_id in &issue.issue.issue.patches {
            ids.insert(patch_id.clone());
        }
    }
    ids.into_iter().collect()
}

async fn fetch_issue_versions(
    client: &dyn MetisClientInterface,
    issue_id: &IssueId,
) -> Result<Vec<Versioned<Issue>>> {
    let response = client
        .list_issue_versions(issue_id)
        .await
        .with_context(|| format!("failed to fetch versions for issue '{issue_id}'"))?;
    Ok(response
        .versions
        .into_iter()
        .map(|record| Versioned::new(record.issue, record.version, record.timestamp))
        .collect())
}

async fn fetch_patch_versions(
    client: &dyn MetisClientInterface,
    patch_id: &PatchId,
) -> Result<Vec<Versioned<metis_common::patches::Patch>>> {
    let response = client
        .list_patch_versions(patch_id)
        .await
        .with_context(|| format!("failed to fetch versions for patch '{patch_id}'"))?;
    Ok(response
        .versions
        .into_iter()
        .map(|record| Versioned::new(record.patch, record.version, record.timestamp))
        .collect())
}

async fn fetch_job_versions(
    client: &dyn MetisClientInterface,
    job_id: &TaskId,
) -> Result<Vec<Versioned<Task>>> {
    let response = client
        .list_job_versions(job_id)
        .await
        .with_context(|| format!("failed to fetch versions for job '{job_id}'"))?;
    Ok(response
        .versions
        .into_iter()
        .map(|record| Versioned::new(record.task, record.version, record.timestamp))
        .collect())
}

async fn fetch_jobs_for_issue(
    client: &dyn MetisClientInterface,
    issue_id: &IssueId,
) -> Result<Vec<JobVersionRecord>> {
    let response = client
        .list_jobs(&SearchJobsQuery::new(
            None,
            Some(issue_id.clone()),
            None,
            None,
        ))
        .await
        .with_context(|| format!("failed to fetch jobs for issue '{issue_id}'"))?;
    Ok(response.jobs)
}

fn filter_activity_entries(
    entries: Vec<ActivityLogEntry>,
    start_time: Option<DateTime<Utc>>,
) -> Vec<ActivityLogEntry> {
    match start_time {
        Some(cutoff) => entries
            .into_iter()
            .filter(|entry| entry.timestamp >= cutoff)
            .collect(),
        None => entries,
    }
}

fn sort_activity_log_entries(entries: &mut [ActivityLogEntry]) {
    entries.sort_by(|a, b| {
        a.timestamp
            .cmp(&b.timestamp)
            .then_with(|| {
                activity_kind_rank(&a.object_kind).cmp(&activity_kind_rank(&b.object_kind))
            })
            .then_with(|| a.object_id.to_string().cmp(&b.object_id.to_string()))
            .then_with(|| a.version.cmp(&b.version))
    });
}

fn activity_kind_rank(kind: &ActivityObjectKind) -> u8 {
    match kind {
        ActivityObjectKind::Issue => 0,
        ActivityObjectKind::Patch => 1,
        ActivityObjectKind::Job => 2,
        _ => u8::MAX,
    }
}

async fn fetch_issues(
    client: &dyn MetisClientInterface,
    id: Option<IssueId>,
    issue_type: Option<IssueType>,
    status: Option<IssueStatus>,
    assignee: Option<String>,
    query: Option<String>,
    graph_filters: Vec<IssueGraphFilter>,
    include_deleted: bool,
) -> Result<Vec<IssueVersionRecord>> {
    if let Some(issue_id) = id {
        let record = client
            .get_issue(&issue_id)
            .await
            .with_context(|| format!("failed to fetch issue '{issue_id}'"))?;

        if let Some(expected_type) = issue_type {
            if record.issue.issue_type != expected_type {
                bail!("Issue '{issue_id}' does not match the requested type.");
            }
        }
        if let Some(expected_status) = status {
            if record.issue.status != expected_status {
                bail!("Issue '{issue_id}' does not match the requested status.");
            }
        }
        if let Some(expected_assignee) = assignee {
            let trimmed_assignee = expected_assignee.trim();
            if trimmed_assignee.is_empty() {
                bail!("Assignee filter must not be empty.");
            }
            match record.issue.assignee.as_deref() {
                Some(current) if current.eq_ignore_ascii_case(trimmed_assignee) => {}
                _ => bail!("Issue '{issue_id}' is not assigned to {trimmed_assignee}."),
            }
        }
        return Ok(vec![record]);
    }

    let trimmed_assignee = match assignee {
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                bail!("Assignee filter must not be empty.");
            }
            Some(trimmed.to_string())
        }
        None => None,
    };

    let trimmed_query = query.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    let include_deleted_opt = if include_deleted { Some(true) } else { None };
    let issues = client
        .list_issues(&SearchIssuesQuery::new(
            issue_type,
            status,
            trimmed_assignee.clone(),
            trimmed_query,
            graph_filters,
            include_deleted_opt,
        ))
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
        if let Some(expected_status) = status {
            if issue.issue.status != expected_status {
                bail!(
                    "Issue {} does not match the requested status.",
                    issue.issue_id
                );
            }
        }
        if let Some(ref expected_assignee) = trimmed_assignee {
            match issue.issue.assignee.as_deref() {
                Some(current) if current.eq_ignore_ascii_case(expected_assignee) => {}
                _ => bail!(
                    "Issue {} is not assigned to {expected_assignee}",
                    issue.issue_id
                ),
            }
        }
    }

    Ok(issues)
}

fn resolve_job_settings(
    current: JobSettings,
    repo_name: Option<String>,
    remote_url: Option<String>,
    image: Option<String>,
    model: Option<String>,
    branch: Option<String>,
    max_retries: Option<u32>,
    secrets: Vec<String>,
    clear_secrets: bool,
    clear_job_settings: bool,
) -> Result<(JobSettings, bool)> {
    if clear_job_settings {
        return Ok((JobSettings::default(), true));
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
    client: &dyn MetisClientInterface,
    current_issue_id: Option<IssueId>,
) -> Result<JobSettings> {
    let Some(issue_id) = current_issue_id else {
        return Ok(JobSettings::default());
    };

    let issue = client
        .get_issue(&issue_id)
        .await
        .with_context(|| format!("failed to fetch issue '{issue_id}'"))?;

    let mut job_settings = JobSettings::default();
    let current = issue.issue.job_settings;
    job_settings.repo_name = current.repo_name;
    job_settings.remote_url = current.remote_url;
    job_settings.image = current.image;
    job_settings.model = current.model;
    job_settings.branch = current.branch;
    job_settings.secrets = current.secrets;

    Ok(job_settings)
}

async fn create_issue(
    client: &dyn MetisClientInterface,
    issue_type: IssueType,
    status: IssueStatus,
    dependencies: Vec<IssueDependency>,
    patches: Vec<PatchId>,
    assignee: Option<String>,
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
) -> Result<IssueVersionRecord> {
    let description = description.trim();
    if description.is_empty() {
        bail!("Issue description must not be empty.");
    }

    let progress = progress
        .map(|value| value.trim().to_string())
        .unwrap_or_default();

    let assignee = match assignee {
        Some(value) => {
            let trimmed = value.trim().to_string();
            if trimmed.is_empty() {
                bail!("Assignee must not be empty.");
            }
            Some(trimmed)
        }
        None => None,
    };

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
    let job_settings =
        (job_settings_requested || !JobSettings::is_default(&job_settings)).then_some(job_settings);

    let issue = Issue::new(
        issue_type,
        description.to_string(),
        creator,
        progress,
        status,
        assignee,
        job_settings,
        Vec::new(),
        dependencies,
        patches,
        false,
    );
    let request = UpsertIssueRequest::new(issue.clone(), None);

    let response = client
        .create_issue(&request)
        .await
        .context("failed to create issue")?;

    Ok(IssueVersionRecord::new(
        response.issue_id,
        response.version,
        Utc::now(),
        issue,
    ))
}

async fn update_issue(
    client: &dyn MetisClientInterface,
    id: IssueId,
    issue_type: Option<IssueType>,
    status: Option<IssueStatus>,
    assignee: Option<String>,
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
    } else if let Some(value) = assignee {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            bail!("Assignee must not be empty.");
        }
        Some(Some(trimmed.to_string()))
    } else {
        None
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

    let job_settings_requested = clear_job_settings
        || repo_name.is_some()
        || remote_url.is_some()
        || image.is_some()
        || model.is_some()
        || branch.is_some()
        || max_retries.is_some()
        || !secrets.is_empty()
        || clear_secrets;

    let no_changes = issue_type.is_none()
        && status.is_none()
        && assignee.is_none()
        && description.is_none()
        && dependencies_update.is_none()
        && patches_update.is_none()
        && progress_update.is_none()
        && !job_settings_requested;
    if no_changes {
        bail!("At least one field must be provided to update.");
    }

    let current = client
        .get_issue(&issue_id)
        .await
        .with_context(|| format!("failed to fetch issue '{issue_id}'"))?;

    let (job_settings, job_settings_changed) = resolve_job_settings(
        current.issue.job_settings.clone(),
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
        Some(current.issue.job_settings.clone())
    };

    let updated_issue = Issue::new(
        issue_type.unwrap_or(current.issue.issue_type),
        description.unwrap_or(current.issue.description),
        current.issue.creator,
        progress_update.unwrap_or(current.issue.progress),
        status.unwrap_or(current.issue.status),
        assignee.unwrap_or(current.issue.assignee),
        job_settings,
        current.issue.todo_list,
        dependencies_update.unwrap_or(current.issue.dependencies),
        patches_update.unwrap_or(current.issue.patches),
        current.issue.deleted,
    );

    let response = client
        .update_issue(
            &issue_id,
            &UpsertIssueRequest::new(updated_issue.clone(), None),
        )
        .await
        .with_context(|| format!("failed to update issue '{issue_id}'"))?;

    Ok(IssueVersionRecord::new(
        response.issue_id,
        response.version,
        Utc::now(),
        updated_issue,
    ))
}

async fn resolve_creator_username(
    client: &dyn MetisClientInterface,
    dependencies: &[IssueDependency],
    current_issue_id: Option<&IssueId>,
) -> Result<Username> {
    let resolve_from_current_issue = || async {
        if let Some(issue_id) = current_issue_id {
            let issue = client
                .get_issue(issue_id)
                .await
                .with_context(|| format!("failed to fetch current issue '{issue_id}'"))?;
            Ok(issue.issue.creator)
        } else {
            bail!("No current issue id available to resolve creator");
        }
    };

    let resolve_from_parent = || async {
        if let Some(parent_id) = dependencies
            .iter()
            .find(|dependency| dependency.dependency_type == IssueDependencyType::ChildOf)
            .map(|dependency| dependency.issue_id.clone())
        {
            let parent = client
                .get_issue(&parent_id)
                .await
                .with_context(|| format!("failed to fetch parent issue '{parent_id}'"))?;
            Ok(parent.issue.creator)
        } else {
            bail!("Failed to resolve authenticated user and no parent issue found");
        }
    };

    let resolve_for_task = || async {
        match resolve_from_current_issue().await {
            Ok(username) => Ok(username),
            Err(_) => resolve_from_parent().await,
        }
    };

    match client
        .whoami()
        .await
        .context("failed to resolve authenticated actor")
    {
        Ok(response) => match response.actor {
            ActorIdentity::User { username } => Ok(username),
            ActorIdentity::Task { .. } => resolve_for_task().await,
            _ => resolve_for_task().await,
        },
        Err(_) => resolve_for_task().await,
    }
}

async fn manage_todo_list(
    client: &dyn MetisClientInterface,
    issue_id: IssueId,
    add: Option<String>,
    done: Option<usize>,
    undone: Option<usize>,
    replace: Option<Vec<String>>,
    output_format: ResolvedOutputFormat,
) -> Result<()> {
    let todo_list = resolve_todo_list(client, &issue_id, add, done, undone, replace).await?;
    print_todo_list(output_format, &issue_id, &todo_list)?;
    Ok(())
}

async fn resolve_todo_list(
    client: &dyn MetisClientInterface,
    issue_id: &IssueId,
    add: Option<String>,
    done: Option<usize>,
    undone: Option<usize>,
    replace: Option<Vec<String>>,
) -> Result<Vec<TodoItem>> {
    if let Some(items) = replace {
        let todo_list = parse_todo_items(items)?;
        let response = client
            .replace_todo_list(issue_id, &ReplaceTodoListRequest::new(todo_list))
            .await
            .with_context(|| format!("failed to replace todo list for issue '{issue_id}'"))?;
        return Ok(response.todo_list);
    }

    if let Some(text) = add {
        let item = parse_todo_item_input(&text)?;
        let response = client
            .add_todo_item(
                issue_id,
                &AddTodoItemRequest::new(item.description, item.is_done),
            )
            .await
            .with_context(|| format!("failed to add todo item for issue '{issue_id}'"))?;
        return Ok(response.todo_list);
    }

    if let Some(item_number) = done {
        let item_number = validate_item_number(item_number)?;
        let response = client
            .set_todo_item_status(issue_id, item_number, &SetTodoItemStatusRequest::new(true))
            .await
            .with_context(|| {
                format!("failed to mark todo item {item_number} done for issue '{issue_id}'")
            })?;
        return Ok(response.todo_list);
    }

    if let Some(item_number) = undone {
        let item_number = validate_item_number(item_number)?;
        let response = client
            .set_todo_item_status(issue_id, item_number, &SetTodoItemStatusRequest::new(false))
            .await
            .with_context(|| {
                format!("failed to mark todo item {item_number} undone for issue '{issue_id}'")
            })?;
        return Ok(response.todo_list);
    }

    let issue = client
        .get_issue(issue_id)
        .await
        .with_context(|| format!("failed to fetch issue '{issue_id}'"))?;
    Ok(issue.issue.todo_list)
}

fn parse_todo_items(raw_items: Vec<String>) -> Result<Vec<TodoItem>> {
    raw_items
        .into_iter()
        .map(|value| parse_todo_item_input(&value))
        .collect()
}

fn validate_item_number(item_number: usize) -> Result<usize> {
    if item_number == 0 {
        bail!("Todo item number must be at least 1.");
    }
    Ok(item_number)
}

fn parse_todo_item_input(raw: &str) -> Result<TodoItem> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("Todo item description must not be empty.");
    }

    let (is_done, description) = if let Some(rest) = trimmed.strip_prefix("[x]") {
        (true, rest)
    } else if let Some(rest) = trimmed.strip_prefix("[X]") {
        (true, rest)
    } else if let Some(rest) = trimmed.strip_prefix("[ ]") {
        (false, rest)
    } else {
        (false, trimmed)
    };

    let description = description.trim().to_string();
    if description.is_empty() {
        bail!("Todo item description must not be empty.");
    }

    Ok(TodoItem::new(description, is_done))
}

fn print_todo_list(
    output_format: ResolvedOutputFormat,
    issue_id: &IssueId,
    todo_list: &[TodoItem],
) -> Result<()> {
    let mut buffer = Vec::new();
    render_todo_list(output_format, issue_id, todo_list, &mut buffer)?;
    io::stdout().write_all(&buffer)?;
    io::stdout().flush()?;
    Ok(())
}

fn render_todo_list(
    output_format: ResolvedOutputFormat,
    issue_id: &IssueId,
    todo_list: &[TodoItem],
    writer: &mut impl Write,
) -> Result<()> {
    if output_format == ResolvedOutputFormat::Jsonl {
        let output = TodoListOutput {
            issue_id,
            todo_list,
        };
        serde_json::to_writer(&mut *writer, &output)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        return Ok(());
    }

    writeln!(writer, "Todos for issue {issue_id}:")?;
    if todo_list.is_empty() {
        writeln!(writer, "  none")?;
        writer.flush()?;
        return Ok(());
    }

    for (index, item) in todo_list.iter().enumerate() {
        let status = if item.is_done { "[x]" } else { "[ ]" };
        let prefix = format!("  {}. {status} ", index + 1);
        let continuation_indent = " ".repeat(prefix.len());
        let mut lines = item.description.lines();

        if let Some(first_line) = lines.next() {
            writeln!(writer, "{prefix}{first_line}")?;
        } else {
            writeln!(writer, "{prefix}-")?;
        }

        for line in lines {
            writeln!(writer, "{continuation_indent}{line}")?;
        }
    }

    writer.flush()?;
    Ok(())
}

fn write_issue_records(format: ResolvedOutputFormat, issues: &[IssueVersionRecord]) -> Result<()> {
    let mut buffer = Vec::new();
    render_issue_records(format, issues, &mut buffer)?;
    io::stdout().write_all(&buffer)?;
    io::stdout().flush()?;
    Ok(())
}

fn parse_issue_graph_filter(raw: &str) -> Result<IssueGraphFilter, String> {
    raw.parse()
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

fn print_issue_description_pretty(
    description: &IssueDescriptionSummary,
    writer: &mut impl Write,
) -> Result<()> {
    writeln!(writer, "{}", colorize_header("Issue"))?;
    write_issue_details_pretty(&description.issue, "  ", true, writer)?;
    writeln!(writer)?;

    writeln!(writer, "{}", colorize_header("Parents:"))?;
    if description.parents.is_empty() {
        writeln!(writer, "  none")?;
    } else {
        for parent in &description.parents {
            writeln!(writer, "  {parent}")?;
        }
    }

    writeln!(writer, "{}", colorize_header("Children (transitive):"))?;
    if description.children.is_empty() {
        writeln!(writer, "  none")?;
    } else {
        for child in &description.children {
            writeln!(writer, "  {child}")?;
        }
    }

    writeln!(writer, "{}", colorize_header("History:"))?;
    if description.activity_log.is_empty() {
        writeln!(writer, "  none")?;
    } else {
        write_activity_log_pretty(&description.activity_log, writer)?;
    }

    writer.flush()?;
    Ok(())
}

fn write_issue_details_pretty(
    issue_with_patches: &IssueWithPatches,
    indent: &str,
    show_todo_list: bool,
    writer: &mut impl Write,
) -> Result<()> {
    let IssueWithPatches {
        issue: issue_record,
        patches: patch_records,
    } = issue_with_patches;
    let Issue {
        issue_type,
        description,
        creator,
        progress,
        status,
        assignee,
        dependencies,
        todo_list,
        ..
    } = &issue_record.issue;

    writeln!(
        writer,
        "{indent}Issue {} ({issue_type}, {status})",
        issue_record.issue_id
    )?;
    writeln!(writer, "{indent}Creator: {}", creator.as_ref())?;
    writeln!(
        writer,
        "{indent}Assignee: {}",
        assignee.as_deref().unwrap_or("-")
    )?;
    writeln!(writer, "{indent}Description:")?;
    if description.trim().is_empty() {
        writeln!(writer, "{indent}  -")?;
    } else {
        for line in description.lines() {
            writeln!(writer, "{indent}  {line}")?;
        }
    }

    writeln!(writer, "{indent}Progress:")?;
    if progress.trim().is_empty() {
        writeln!(writer, "{indent}  -")?;
    } else {
        for line in progress.lines() {
            writeln!(writer, "{indent}  {line}")?;
        }
    }

    if show_todo_list {
        write_todo_list(indent, todo_list, writer)?;
    }

    if dependencies.is_empty() {
        writeln!(writer, "{indent}Dependencies: none")?;
    } else {
        writeln!(writer, "{indent}Dependencies:")?;
        for dependency in dependencies {
            writeln!(
                writer,
                "{indent}  - {} {}",
                dependency.dependency_type, dependency.issue_id
            )?;
        }
    }

    if patch_records.is_empty() {
        writeln!(writer, "{indent}Patches: none")?;
    } else {
        writeln!(writer, "{indent}Patches:")?;
        for patch in patch_records {
            let status = patch.patch.status;
            let title = if patch.patch.title.is_empty() {
                "(untitled)"
            } else {
                patch.patch.title.as_str()
            };
            writeln!(
                writer,
                "{indent}  - {title} ({}) [{status}]",
                patch.patch_id
            )?;
            writeln!(writer, "{indent}    Description:")?;
            if patch.patch.description.trim().is_empty() {
                writeln!(writer, "{indent}      -")?;
            } else {
                for line in patch.patch.description.lines() {
                    writeln!(writer, "{indent}      {line}")?;
                }
            }
            write_patch_review_summary(&patch.patch.reviews, indent, writer)?;
        }
    }

    Ok(())
}

fn write_activity_log_pretty(
    entries: &[ActivityLogEntrySummary],
    writer: &mut impl Write,
) -> Result<()> {
    for (index, entry) in entries.iter().enumerate() {
        write_activity_log_entry_pretty(entry, "  ", writer)?;
        if index + 1 < entries.len() {
            writeln!(writer)?;
        }
    }
    Ok(())
}

fn write_activity_log_entry_pretty(
    entry: &ActivityLogEntrySummary,
    indent: &str,
    writer: &mut impl Write,
) -> Result<()> {
    let timestamp = entry.timestamp.to_rfc3339_opts(SecondsFormat::Secs, true);
    let kind_label = match entry.object_kind {
        ActivityObjectKind::Issue => "Issue",
        ActivityObjectKind::Patch => "Patch",
        ActivityObjectKind::Job => "Job",
        _ => "Activity",
    };
    let event_label = match entry.event {
        ActivityEventSummary::Created => "created",
        ActivityEventSummary::Updated { .. } => "updated",
    };

    writeln!(
        writer,
        "{indent}{} {} {} v{} {}",
        colorize_dimmed(&timestamp),
        colorize_bold(kind_label),
        entry.object_id,
        entry.version,
        event_label
    )?;

    let detail_indent = format!("{indent}  ");
    write_activity_object_summary(&entry.object, &entry.event, &detail_indent, writer)?;

    Ok(())
}

fn write_activity_object_summary(
    object: &ActivityObjectSummary,
    event: &ActivityEventSummary,
    indent: &str,
    writer: &mut impl Write,
) -> Result<()> {
    let change_map = match event {
        ActivityEventSummary::Updated { changes, .. } => changes
            .iter()
            .map(|change| (change.field.as_str(), change))
            .collect::<HashMap<_, _>>(),
        ActivityEventSummary::Created => HashMap::new(),
    };

    match object {
        ActivityObjectSummary::Issue {
            issue_type,
            status,
            description,
            assignee,
            progress,
        } => {
            write_activity_scalar_field(
                "Type",
                &Value::String(issue_type.to_string()),
                change_map.get("Type").copied(),
                indent,
                writer,
            )?;
            write_activity_scalar_field(
                "Status",
                &Value::String(status.to_string()),
                change_map.get("Status").copied(),
                indent,
                writer,
            )?;
            write_activity_optional_scalar_field(
                "Assignee",
                assignee.as_deref(),
                change_map.get("Assignee").copied(),
                indent,
                writer,
            )?;
            write_activity_multiline_field(
                "Description",
                description,
                change_map.get("Description").copied(),
                indent,
                writer,
            )?;
            write_activity_multiline_field(
                "Progress",
                progress,
                change_map.get("Progress").copied(),
                indent,
                writer,
            )?;
        }
        ActivityObjectSummary::Patch {
            title,
            description,
            status,
            repo,
            created_by_job,
            reviews,
        } => {
            write_activity_scalar_field(
                "Title",
                &Value::String(title.clone()),
                change_map.get("Title").copied(),
                indent,
                writer,
            )?;
            write_activity_scalar_field(
                "Status",
                &Value::String(status.to_string()),
                change_map.get("Status").copied(),
                indent,
                writer,
            )?;
            write_activity_scalar_field(
                "Repo",
                &Value::String(repo.to_string()),
                change_map.get("Repo").copied(),
                indent,
                writer,
            )?;
            write_activity_optional_scalar_field(
                "Created By Job",
                created_by_job.as_ref().map(|id| id.to_string()).as_deref(),
                change_map.get("Created By Job").copied(),
                indent,
                writer,
            )?;
            write_activity_multiline_field(
                "Description",
                description,
                change_map.get("Description").copied(),
                indent,
                writer,
            )?;
            write_activity_reviews(reviews, indent, writer)?;
        }
        ActivityObjectSummary::Job { status } => {
            write_activity_scalar_field(
                "Status",
                &Value::String(format_job_status(*status).to_string()),
                change_map.get("Status").copied(),
                indent,
                writer,
            )?;
        }
    }

    if let ActivityEventSummary::Updated { other_changes, .. } = event {
        if !other_changes.is_empty() {
            let joined = other_changes.join(", ");
            writeln!(writer, "{indent}Other changes: {}", joined.dimmed())?;
        }
    }

    Ok(())
}

fn write_activity_scalar_field(
    label: &str,
    current: &Value,
    change: Option<&ActivityFieldChangeSummary>,
    indent: &str,
    writer: &mut impl Write,
) -> Result<()> {
    if let Some(change) = change {
        writeln!(
            writer,
            "{indent}{label}: {} -> {}",
            format_struck_value_for_label(label, &change.before),
            format_activity_value_pretty_for_label(label, &change.after)
        )?;
    } else {
        writeln!(
            writer,
            "{indent}{label}: {}",
            format_activity_value_pretty_for_label(label, current)
        )?;
    }
    Ok(())
}

fn write_activity_optional_scalar_field(
    label: &str,
    current: Option<&str>,
    change: Option<&ActivityFieldChangeSummary>,
    indent: &str,
    writer: &mut impl Write,
) -> Result<()> {
    if let Some(change) = change {
        writeln!(
            writer,
            "{indent}{label}: {} -> {}",
            format_struck_value_for_label(label, &change.before),
            format_activity_value_pretty_for_label(label, &change.after)
        )?;
    } else {
        writeln!(writer, "{indent}{label}: {}", current.unwrap_or("-"))?;
    }
    Ok(())
}

fn write_activity_multiline_field(
    label: &str,
    value: &str,
    change: Option<&ActivityFieldChangeSummary>,
    indent: &str,
    writer: &mut impl Write,
) -> Result<()> {
    if let Some(change) = change {
        writeln!(
            writer,
            "{indent}{label}: {} -> {}",
            format_struck_value_for_label(label, &change.before),
            format_activity_value_pretty_for_label(label, &change.after)
        )?;
        return Ok(());
    }

    writeln!(writer, "{indent}{label}:")?;
    if value.trim().is_empty() {
        writeln!(writer, "{indent}  -")?;
    } else {
        for line in value.lines() {
            writeln!(writer, "{indent}  {line}")?;
        }
    }
    Ok(())
}

fn write_activity_reviews(
    reviews: &[ReviewSummary],
    indent: &str,
    writer: &mut impl Write,
) -> Result<()> {
    if reviews.is_empty() {
        writeln!(writer, "{indent}Reviews: none")?;
        return Ok(());
    }

    writeln!(writer, "{indent}Reviews:")?;
    for review in reviews {
        writeln!(
            writer,
            "{indent}  - {}: {} ({})",
            review.author,
            colorize_review_decision(review.is_approved),
            review.contents
        )?;
    }
    Ok(())
}

fn format_activity_value_pretty(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Null => "-".to_string(),
        _ => format_activity_value(value),
    }
}

fn format_activity_value_pretty_for_label(label: &str, value: &Value) -> String {
    let rendered = format_activity_value_pretty(value);
    if label == "Status" {
        rendered.replace('_', "-")
    } else {
        rendered
    }
}

fn format_struck_value_for_label(label: &str, value: &Value) -> String {
    let rendered = format_activity_value_pretty_for_label(label, value);
    if supports_ansi() {
        format!("\x1b[9m{rendered}\x1b[0m")
    } else {
        rendered
    }
}

fn supports_ansi() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    !matches!(std::env::var("TERM"), Ok(term) if term == "dumb")
}

fn colorize_header(text: &str) -> String {
    if supports_ansi() {
        text.bold().bright_blue().to_string()
    } else {
        text.to_string()
    }
}

fn colorize_dimmed(text: &str) -> String {
    if supports_ansi() {
        text.dimmed().to_string()
    } else {
        text.to_string()
    }
}

fn colorize_bold(text: &str) -> String {
    if supports_ansi() {
        text.bold().to_string()
    } else {
        text.to_string()
    }
}

fn colorize_review_decision(is_approved: bool) -> String {
    let text = if is_approved {
        "approved"
    } else {
        "changes requested"
    };
    if supports_ansi() {
        if is_approved {
            text.green().to_string()
        } else {
            text.red().to_string()
        }
    } else {
        text.to_string()
    }
}

fn format_job_status(status: Status) -> &'static str {
    match status {
        Status::Created => "created",
        Status::Pending => "pending",
        Status::Running => "running",
        Status::Complete => "complete",
        Status::Failed => "failed",
        Status::Unknown => "unknown",
        _ => "unknown",
    }
}

fn format_activity_value(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "<unavailable>".to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PatchReviewSummary {
    approvals: usize,
    change_requests: usize,
    latest_review: ReviewSnapshot,
    reviewer_statuses: Vec<ReviewSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReviewSnapshot {
    author: String,
    is_approved: bool,
    submitted_at: Option<DateTime<Utc>>,
}

fn write_patch_review_summary(
    reviews: &[Review],
    indent: &str,
    writer: &mut impl Write,
) -> Result<()> {
    match build_review_summary(reviews) {
        Some(summary) => {
            writeln!(writer, "{indent}    Reviews:")?;
            writeln!(
                writer,
                "{indent}      Latest: {} by {}{}",
                review_decision(summary.latest_review.is_approved),
                summary.latest_review.author,
                format_timestamp(summary.latest_review.submitted_at.as_ref())
            )?;
            writeln!(
                writer,
                "{indent}      Counts: {} {}, {} {}",
                summary.approvals,
                if summary.approvals == 1 {
                    "approval"
                } else {
                    "approvals"
                },
                summary.change_requests,
                if summary.change_requests == 1 {
                    "change request"
                } else {
                    "change requests"
                }
            )?;
            writeln!(writer, "{indent}      Reviewers:")?;
            for reviewer in summary.reviewer_statuses {
                writeln!(
                    writer,
                    "{indent}        - {}: {}{}",
                    reviewer.author,
                    review_decision(reviewer.is_approved),
                    format_timestamp(reviewer.submitted_at.as_ref())
                )?;
            }
        }
        None => {
            writeln!(writer, "{indent}    Reviews: none")?;
        }
    }

    Ok(())
}

fn build_review_summary(reviews: &[Review]) -> Option<PatchReviewSummary> {
    if reviews.is_empty() {
        return None;
    }

    let mut approvals = 0;
    let mut change_requests = 0;
    let mut latest_review_index: Option<usize> = None;
    let mut latest_by_author = HashMap::new();

    for (index, review) in reviews.iter().enumerate() {
        if review.is_approved {
            approvals += 1;
        } else {
            change_requests += 1;
        }

        match latest_review_index {
            Some(current_index) if !is_more_recent_review(current_index, index, reviews) => {}
            _ => latest_review_index = Some(index),
        }

        match latest_by_author.get(review.author.as_str()) {
            Some(&current_index) if !is_more_recent_review(current_index, index, reviews) => {}
            _ => {
                latest_by_author.insert(review.author.as_str(), index);
            }
        }
    }

    let mut reviewer_statuses: Vec<ReviewSnapshot> = latest_by_author
        .into_iter()
        .map(|(author, index)| {
            let review = &reviews[index];
            ReviewSnapshot {
                author: author.to_string(),
                is_approved: review.is_approved,
                submitted_at: review.submitted_at,
            }
        })
        .collect();
    reviewer_statuses.sort_by(|a, b| a.author.cmp(&b.author));

    let latest_index = latest_review_index.expect("non-empty reviews always set latest index");
    let latest = &reviews[latest_index];

    Some(PatchReviewSummary {
        approvals,
        change_requests,
        latest_review: ReviewSnapshot {
            author: latest.author.clone(),
            is_approved: latest.is_approved,
            submitted_at: latest.submitted_at,
        },
        reviewer_statuses,
    })
}

fn is_more_recent_review(current_index: usize, candidate_index: usize, reviews: &[Review]) -> bool {
    let current = &reviews[current_index];
    let candidate = &reviews[candidate_index];
    match (&current.submitted_at, &candidate.submitted_at) {
        (Some(current_time), Some(candidate_time)) => candidate_time > current_time,
        (None, Some(_)) => true,
        (Some(_), None) => false,
        (None, None) => candidate_index > current_index,
    }
}

fn review_decision(is_approved: bool) -> &'static str {
    if is_approved {
        "approved"
    } else {
        "changes requested"
    }
}

fn write_todo_list(indent: &str, todo_list: &[TodoItem], writer: &mut impl Write) -> Result<()> {
    writeln!(writer, "{indent}Todos:")?;
    if todo_list.is_empty() {
        writeln!(writer, "{indent}  none")?;
        return Ok(());
    }

    for (index, item) in todo_list.iter().enumerate() {
        let status = if item.is_done { "[x]" } else { "[ ]" };
        let prefix = format!("{indent}  {}. {status} ", index + 1);
        let continuation_indent = " ".repeat(prefix.len());
        let mut lines = item.description.lines();

        if let Some(first_line) = lines.next() {
            writeln!(writer, "{prefix}{first_line}")?;
        } else {
            writeln!(writer, "{prefix}-")?;
        }

        for line in lines {
            writeln!(writer, "{continuation_indent}{line}")?;
        }
    }

    Ok(())
}

fn format_timestamp(timestamp: Option<&DateTime<Utc>>) -> String {
    timestamp
        .map(|value| value.to_rfc3339_opts(SecondsFormat::Secs, true))
        .map(|value| format!(" @ {value}"))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MetisClient;
    use crate::test_utils::ids::{issue_id, patch_id, task_id};
    use chrono::{Duration, TimeZone, Utc};
    use httpmock::prelude::*;
    use metis_common::issues::{
        AddTodoItemRequest, Issue, IssueGraphSelector, IssueGraphWildcard, IssueVersionRecord,
        JobSettings, ListIssueVersionsResponse, ListIssuesResponse, ReplaceTodoListRequest,
        SetTodoItemStatusRequest, TodoItem, TodoListResponse, UpsertIssueRequest,
        UpsertIssueResponse,
    };
    use metis_common::{
        jobs::{BundleSpec, ListJobsResponse, Task},
        patches::{ListPatchVersionsResponse, Patch, PatchStatus, PatchVersionRecord, Review},
        task_status::Status,
        users::Username,
        whoami::{ActorIdentity, WhoAmIResponse},
        PatchId, RepoName, TaskId,
    };
    use reqwest::Client as HttpClient;
    use std::collections::HashMap;
    use std::str::FromStr;

    const TEST_METIS_TOKEN: &str = "test-metis-token";

    fn sample_diff() -> String {
        "--- a/file.txt\n+++ b/file.txt\n@@\n-old\n+new\n".to_string()
    }

    fn sample_repo_name() -> RepoName {
        RepoName::from_str("dourolabs/example").unwrap()
    }

    fn sample_job_settings() -> JobSettings {
        let mut job_settings = JobSettings::default();
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

    fn metis_client(server: &MockServer) -> MetisClient {
        MetisClient::with_http_client(server.base_url(), TEST_METIS_TOKEN, HttpClient::new())
            .unwrap()
    }

    fn strip_ansi_codes(input: &str) -> String {
        let mut output = String::with_capacity(input.len());
        let mut chars = input.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\u{1b}' && matches!(chars.peek(), Some('[')) {
                chars.next();
                for next in chars.by_ref() {
                    if next == 'm' {
                        break;
                    }
                }
                continue;
            }
            output.push(ch);
        }
        output
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
                description.into(),
                empty_user(),
                String::new(),
                status,
                assignee.map(str::to_string),
                None,
                Vec::new(),
                dependencies,
                patches,
                false,
            ),
        )
    }

    #[tokio::test]
    async fn list_issues_filters_by_query_and_prints_jsonl() {
        let server = MockServer::start();
        let client = metis_client(&server);
        let issues_response = ListIssuesResponse::new(vec![IssueVersionRecord::new(
            issue_id("i-1"),
            0,
            Utc::now(),
            Issue::new(
                IssueType::Bug,
                "First issue".into(),
                empty_user(),
                String::new(),
                IssueStatus::Open,
                None,
                None,
                Vec::new(),
                vec![],
                Vec::new(),
                false,
            ),
        )]);
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
            Some(IssueStatus::Open),
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
        render_issue_records(ResolvedOutputFormat::Jsonl, &issues, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        let first_id = issue_id("i-1").to_string();
        let second_id = issue_id("i-2").to_string();
        assert!(output.contains(&format!("\"issue_id\":\"{first_id}\"")));
        assert!(!output.contains(&format!("\"issue_id\":\"{second_id}\"")));
    }

    #[tokio::test]
    async fn list_issues_by_id_returns_single_issue() {
        let server = MockServer::start();
        let client = metis_client(&server);
        let issue_id = issue_id("i-123");
        let issue_record = IssueVersionRecord::new(
            issue_id.clone(),
            0,
            Utc::now(),
            Issue::new(
                IssueType::Task,
                "Edge case bug".into(),
                empty_user(),
                String::new(),
                IssueStatus::InProgress,
                None,
                None,
                Vec::new(),
                vec![],
                Vec::new(),
                false,
            ),
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
            Some(IssueStatus::InProgress),
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
        let client = metis_client(&server);
        let issues_response = ListIssuesResponse::new(vec![IssueVersionRecord::new(
            issue_id("i-7"),
            0,
            Utc::now(),
            Issue::new(
                IssueType::Task,
                "Edge case bug".into(),
                empty_user(),
                String::new(),
                IssueStatus::Open,
                Some("owner-a".into()),
                None,
                Vec::new(),
                vec![],
                Vec::new(),
                false,
            ),
        )]);
        let list_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/issues")
                .query_param("assignee", "OWNER-A");
            then.status(200).json_body_obj(&issues_response);
        });

        let issues = fetch_issues(
            &client,
            None,
            None,
            None,
            Some("OWNER-A".into()),
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
    async fn list_issues_includes_graph_filters_in_query() {
        let server = MockServer::start();
        let client = metis_client(&server);
        let filters = vec![
            parse_issue_graph_filter("*:child-of:i-abcd").unwrap(),
            parse_issue_graph_filter("i-efgh:blocked-on:**").unwrap(),
        ];
        let graph_query = filters
            .iter()
            .map(|filter| filter.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let list_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/issues")
                .query_param("graph", graph_query.as_str());
            then.status(200)
                .json_body_obj(&ListIssuesResponse::new(vec![]));
        });

        let _ = fetch_issues(
            &client,
            None,
            None,
            None,
            None,
            None,
            filters.clone(),
            false,
        )
        .await
        .unwrap();

        list_mock.assert();
        assert_eq!(list_mock.hits(), 1);
    }

    #[tokio::test]
    async fn describe_issue_collects_related_issues_and_children() {
        let server = MockServer::start();
        let client = metis_client(&server);
        let root_id = issue_id("i-root");
        let parent_id = issue_id("i-parent");
        let root_patch_id = patch_id("p-root");
        let parent_patch_id = patch_id("p-parent");
        let child_patch_id = patch_id("p-child");

        let parent_issue = api_issue_record(
            "i-parent",
            IssueType::Task,
            "Parent issue",
            IssueStatus::Open,
            None,
            vec![],
            vec![parent_patch_id.clone()],
        );

        let root_issue = api_issue_record(
            "i-root",
            IssueType::Task,
            "Root issue",
            IssueStatus::Open,
            Some("owner"),
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
            vec![root_patch_id.clone()],
        );

        let child_issue = api_issue_record(
            "i-child",
            IssueType::Bug,
            "Child issue",
            IssueStatus::InProgress,
            None,
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                root_id.clone(),
            )],
            vec![child_patch_id.clone()],
        );

        let root_issue_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{root_id}").as_str());
            then.status(200).json_body_obj(&root_issue);
        });
        let parent_issue_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{parent_id}").as_str());
            then.status(200).json_body_obj(&parent_issue);
        });
        let graph_query = IssueGraphFilter::new(
            IssueGraphSelector::Wildcard(IssueGraphWildcard::Transitive),
            IssueDependencyType::ChildOf,
            IssueGraphSelector::Issue(root_id.clone()),
        )
        .unwrap()
        .to_string();
        let list_children_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/issues")
                .query_param("graph", graph_query.as_str());
            then.status(200)
                .json_body_obj(&ListIssuesResponse::new(vec![child_issue.clone()]));
        });
        let root_patch_record = PatchVersionRecord::new(
            root_patch_id.clone(),
            0,
            Utc::now(),
            Patch::new(
                "root patch".into(),
                "desc".into(),
                sample_diff(),
                Default::default(),
                false,
                None,
                Vec::new(),
                sample_repo_name(),
                None,
                false,
            ),
        );
        let parent_patch_record = PatchVersionRecord::new(
            parent_patch_id.clone(),
            0,
            Utc::now(),
            Patch::new(
                "parent patch".into(),
                "desc".into(),
                sample_diff(),
                Default::default(),
                false,
                None,
                Vec::new(),
                sample_repo_name(),
                None,
                false,
            ),
        );
        let child_patch_record = PatchVersionRecord::new(
            child_patch_id.clone(),
            0,
            Utc::now(),
            Patch::new(
                "child patch".into(),
                "desc".into(),
                sample_diff(),
                Default::default(),
                false,
                None,
                Vec::new(),
                sample_repo_name(),
                None,
                false,
            ),
        );
        let version_timestamp = Utc.with_ymd_and_hms(2024, 2, 1, 12, 0, 0).unwrap();
        let root_versions = ListIssueVersionsResponse::new(vec![IssueVersionRecord::new(
            root_id.clone(),
            1,
            version_timestamp,
            root_issue.issue.clone(),
        )]);
        let parent_versions = ListIssueVersionsResponse::new(vec![IssueVersionRecord::new(
            parent_id.clone(),
            1,
            version_timestamp,
            parent_issue.issue.clone(),
        )]);
        let child_versions = ListIssueVersionsResponse::new(vec![IssueVersionRecord::new(
            child_issue.issue_id.clone(),
            1,
            version_timestamp,
            child_issue.issue.clone(),
        )]);
        let root_patch_versions = ListPatchVersionsResponse::new(vec![PatchVersionRecord::new(
            root_patch_id.clone(),
            1,
            version_timestamp,
            root_patch_record.patch.clone(),
        )]);
        let parent_patch_versions = ListPatchVersionsResponse::new(vec![PatchVersionRecord::new(
            parent_patch_id.clone(),
            1,
            version_timestamp,
            parent_patch_record.patch.clone(),
        )]);
        let child_patch_versions = ListPatchVersionsResponse::new(vec![PatchVersionRecord::new(
            child_patch_id.clone(),
            1,
            version_timestamp,
            child_patch_record.patch.clone(),
        )]);
        let root_patch_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/patches/{root_patch_id}").as_str());
            then.status(200).json_body_obj(&root_patch_record);
        });
        let parent_patch_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/patches/{parent_patch_id}").as_str());
            then.status(200).json_body_obj(&parent_patch_record);
        });
        let child_patch_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/patches/{child_patch_id}").as_str());
            then.status(200).json_body_obj(&child_patch_record);
        });
        let root_versions_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{root_id}/versions").as_str());
            then.status(200).json_body_obj(&root_versions);
        });
        let parent_versions_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{parent_id}/versions").as_str());
            then.status(200).json_body_obj(&parent_versions);
        });
        let child_versions_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{}/versions", child_issue.issue_id).as_str());
            then.status(200).json_body_obj(&child_versions);
        });
        let root_patch_versions_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/patches/{root_patch_id}/versions").as_str());
            then.status(200).json_body_obj(&root_patch_versions);
        });
        let parent_patch_versions_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/patches/{parent_patch_id}/versions").as_str());
            then.status(200).json_body_obj(&parent_patch_versions);
        });
        let child_patch_versions_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/patches/{child_patch_id}/versions").as_str());
            then.status(200).json_body_obj(&child_patch_versions);
        });
        let list_jobs_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/jobs/")
                .query_param("spawned_from", root_id.as_ref());
            then.status(200)
                .json_body_obj(&ListJobsResponse::new(Vec::new()));
        });

        let description = collect_issue_description(&client, root_id.clone())
            .await
            .unwrap();

        root_issue_mock.assert();
        parent_issue_mock.assert();
        list_children_mock.assert();
        root_patch_mock.assert();
        parent_patch_mock.assert();
        child_patch_mock.assert();
        root_versions_mock.assert();
        parent_versions_mock.assert();
        child_versions_mock.assert();
        root_patch_versions_mock.assert();
        parent_patch_versions_mock.assert();
        child_patch_versions_mock.assert();
        list_jobs_mock.assert();
        assert_eq!(root_issue_mock.hits(), 1);
        assert_eq!(parent_issue_mock.hits(), 1);
        assert_eq!(list_children_mock.hits(), 1);
        assert_eq!(root_patch_mock.hits(), 1);
        assert_eq!(parent_patch_mock.hits(), 1);
        assert_eq!(child_patch_mock.hits(), 1);
        assert_eq!(root_versions_mock.hits(), 1);
        assert_eq!(parent_versions_mock.hits(), 1);
        assert_eq!(child_versions_mock.hits(), 1);
        assert_eq!(root_patch_versions_mock.hits(), 1);
        assert_eq!(parent_patch_versions_mock.hits(), 1);
        assert_eq!(child_patch_versions_mock.hits(), 1);
        assert_eq!(list_jobs_mock.hits(), 1);
        assert_eq!(
            description.issue,
            IssueWithPatches {
                issue: root_issue,
                patches: vec![root_patch_record]
            }
        );
        assert_eq!(
            description.parents,
            vec![IssueWithPatches {
                issue: parent_issue,
                patches: vec![parent_patch_record]
            }]
        );
        assert_eq!(
            description.children,
            vec![IssueWithPatches {
                issue: child_issue,
                patches: vec![child_patch_record]
            }]
        );
    }

    #[tokio::test]
    async fn create_issue_submits_issue_record() {
        let server = MockServer::start();
        let client = metis_client(&server);

        let patch_ids = vec![patch_id("p-123")];
        let create_request = UpsertIssueRequest::new(
            Issue::new(
                IssueType::MergeRequest,
                "New issue description".into(),
                Username::from("creator-a"),
                "Initial notes".into(),
                IssueStatus::Closed,
                Some("team-a".into()),
                None,
                Vec::new(),
                Vec::new(),
                patch_ids.clone(),
                false,
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
            IssueStatus::Closed,
            Vec::new(),
            patch_ids.clone(),
            Some("team-a".into()),
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
        )
        .await
        .unwrap();

        create_mock.assert();
        assert_eq!(create_mock.hits(), 1);
    }

    #[tokio::test]
    async fn create_issue_sets_job_settings() {
        let server = MockServer::start();
        let client = metis_client(&server);

        let mut job_settings = JobSettings::default();
        job_settings.repo_name = Some(sample_repo_name());
        job_settings.remote_url = Some("https://example.com/service.git".into());
        job_settings.image = Some("worker:latest".into());
        job_settings.branch = Some("feature/job-settings".into());
        job_settings.max_retries = Some(4);
        let create_request = UpsertIssueRequest::new(
            Issue::new(
                IssueType::MergeRequest,
                "New issue description".into(),
                Username::from("creator-a"),
                "Initial notes".into(),
                IssueStatus::Closed,
                Some("team-a".into()),
                Some(job_settings.clone()),
                Vec::new(),
                Vec::new(),
                vec![],
                false,
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
            IssueStatus::Closed,
            Vec::new(),
            vec![],
            Some("team-a".into()),
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
        )
        .await
        .unwrap();

        create_mock.assert();
        assert_eq!(create_mock.hits(), 1);
    }

    #[tokio::test]
    async fn create_issue_inherits_job_settings_from_current_issue() {
        let server = MockServer::start();
        let client = metis_client(&server);

        let current_issue_id = issue_id("i-current");
        let mut inherited_settings = JobSettings::default();
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
                "Existing issue".into(),
                empty_user(),
                String::new(),
                IssueStatus::Open,
                None,
                Some(inherited_settings.clone()),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                false,
            ),
        );
        let current_issue_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{current_issue_id}").as_str());
            then.status(200).json_body_obj(&current_issue);
        });
        let create_request = UpsertIssueRequest::new(
            Issue::new(
                IssueType::MergeRequest,
                "New issue description".into(),
                Username::from("creator-a"),
                "Initial notes".into(),
                IssueStatus::Open,
                None,
                Some(inherited_settings),
                Vec::new(),
                Vec::new(),
                vec![],
                false,
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
            IssueStatus::Open,
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
        let client = metis_client(&server);

        let current_issue_id = issue_id("i-current");
        let mut inherited_settings = JobSettings::default();
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
                "Existing issue".into(),
                empty_user(),
                String::new(),
                IssueStatus::Open,
                None,
                Some(inherited_settings.clone()),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                false,
            ),
        );
        let current_issue_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{current_issue_id}").as_str());
            then.status(200).json_body_obj(&current_issue);
        });

        let mut expected_settings = JobSettings::default();
        expected_settings.repo_name = Some(RepoName::from_str("dourolabs/override").unwrap());
        expected_settings.remote_url = inherited_settings.remote_url.clone();
        expected_settings.image = Some("custom:tag".into());
        expected_settings.branch = Some("override-branch".into());
        let create_request = UpsertIssueRequest::new(
            Issue::new(
                IssueType::MergeRequest,
                "New issue description".into(),
                Username::from("creator-a"),
                "Initial notes".into(),
                IssueStatus::Open,
                None,
                Some(expected_settings.clone()),
                Vec::new(),
                Vec::new(),
                vec![],
                false,
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
            IssueStatus::Open,
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
        let client = metis_client(&server);

        let mut job_settings = JobSettings::default();
        job_settings.secrets = Some(vec!["my-api-secret".into(), "my-db-secret".into()]);
        let create_request = UpsertIssueRequest::new(
            Issue::new(
                IssueType::Task,
                "Issue with secrets".into(),
                Username::from("creator-a"),
                String::new(),
                IssueStatus::Open,
                None,
                Some(job_settings.clone()),
                Vec::new(),
                Vec::new(),
                vec![],
                false,
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
            IssueStatus::Open,
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
        )
        .await
        .unwrap();

        create_mock.assert();
        assert_eq!(create_mock.hits(), 1);
    }

    #[tokio::test]
    async fn create_issue_inherits_secrets_from_current_issue() {
        let server = MockServer::start();
        let client = metis_client(&server);

        let current_issue_id = issue_id("i-current");
        let mut inherited_settings = JobSettings::default();
        inherited_settings.secrets = Some(vec!["inherited-secret".into()]);
        let current_issue = IssueVersionRecord::new(
            current_issue_id.clone(),
            0,
            Utc::now(),
            Issue::new(
                IssueType::Task,
                "Parent issue".into(),
                empty_user(),
                String::new(),
                IssueStatus::Open,
                None,
                Some(inherited_settings.clone()),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                false,
            ),
        );
        let current_issue_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{current_issue_id}").as_str());
            then.status(200).json_body_obj(&current_issue);
        });
        let create_request = UpsertIssueRequest::new(
            Issue::new(
                IssueType::Task,
                "Child issue".into(),
                Username::from("creator-a"),
                String::new(),
                IssueStatus::Open,
                None,
                Some(inherited_settings),
                Vec::new(),
                Vec::new(),
                vec![],
                false,
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
            IssueStatus::Open,
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
        let client = metis_client(&server);
        assert!(create_issue(
            &client,
            IssueType::Bug,
            IssueStatus::Open,
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
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn create_issue_rejects_empty_assignee() {
        let server = MockServer::start();
        let client = metis_client(&server);
        assert!(create_issue(
            &client,
            IssueType::Bug,
            IssueStatus::Open,
            vec![],
            Vec::new(),
            Some("   ".into()),
            empty_user(),
            "Valid description".into(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Vec::new(),
            None,
        )
        .await
        .is_err());
    }

    #[test]
    fn parse_issue_dependency_parses_type_and_id() {
        let dependency = parse_issue_dependency("child-of:i-abcd").unwrap();
        assert_eq!(dependency.dependency_type, IssueDependencyType::ChildOf);
        assert_eq!(dependency.issue_id, issue_id("i-abcd"));
    }

    #[test]
    fn parse_issue_graph_filter_parses_format() {
        let filter = parse_issue_graph_filter("*:child-of:i-abcd").unwrap();
        assert!(matches!(
            filter.lhs,
            IssueGraphSelector::Wildcard(IssueGraphWildcard::Immediate)
        ));
        assert_eq!(filter.literal_issue_id(), &issue_id("i-abcd"));
    }

    #[test]
    fn parse_issue_graph_filter_rejects_invalid_shapes() {
        assert!(parse_issue_graph_filter("i-abcd:child-of:i-efgh").is_err());
    }

    #[tokio::test]
    async fn resolve_creator_username_uses_whoami_user() {
        let server = MockServer::start();
        let client = metis_client(&server);
        let whoami_response = WhoAmIResponse::new(ActorIdentity::User {
            username: Username::from("creator-a"),
        });
        let whoami_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/whoami");
            then.status(200).json_body_obj(&whoami_response);
        });

        let username = resolve_creator_username(&client, &[], None).await.unwrap();

        assert_eq!(username, Username::from("creator-a"));
        whoami_mock.assert();
        assert_eq!(whoami_mock.hits(), 1);
    }

    #[tokio::test]
    async fn resolve_creator_username_falls_back_to_parent_for_task() {
        let server = MockServer::start();
        let client = metis_client(&server);
        let parent_id = issue_id("i-parent");
        let whoami_response = WhoAmIResponse::new(ActorIdentity::Task {
            task_id: TaskId::from_str("t-abcd").unwrap(),
        });
        let whoami_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/whoami");
            then.status(200).json_body_obj(&whoami_response);
        });
        let parent_issue = IssueVersionRecord::new(
            parent_id.clone(),
            0,
            Utc::now(),
            Issue::new(
                IssueType::Task,
                "Parent issue".into(),
                Username::from("parent-creator"),
                String::new(),
                IssueStatus::Open,
                None,
                None,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                false,
            ),
        );
        let parent_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{parent_id}").as_str());
            then.status(200).json_body_obj(&parent_issue);
        });

        let dependencies = vec![IssueDependency::new(
            IssueDependencyType::ChildOf,
            parent_id,
        )];
        let username = resolve_creator_username(&client, &dependencies, None)
            .await
            .unwrap();

        assert_eq!(username, Username::from("parent-creator"));
        whoami_mock.assert();
        parent_mock.assert();
        assert_eq!(whoami_mock.hits(), 1);
        assert_eq!(parent_mock.hits(), 1);
    }

    #[tokio::test]
    async fn resolve_creator_username_resolves_from_current_issue_for_task() {
        let server = MockServer::start();
        let client = metis_client(&server);
        let current_id = issue_id("i-current");
        let whoami_response = WhoAmIResponse::new(ActorIdentity::Task {
            task_id: TaskId::from_str("t-abcd").unwrap(),
        });
        let whoami_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/whoami");
            then.status(200).json_body_obj(&whoami_response);
        });
        let current_issue = IssueVersionRecord::new(
            current_id.clone(),
            0,
            Utc::now(),
            Issue::new(
                IssueType::Task,
                "Current issue".into(),
                Username::from("current-creator"),
                String::new(),
                IssueStatus::InProgress,
                None,
                None,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                false,
            ),
        );
        let current_issue_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{current_id}").as_str());
            then.status(200).json_body_obj(&current_issue);
        });

        let username = resolve_creator_username(&client, &[], Some(&current_id))
            .await
            .unwrap();

        assert_eq!(username, Username::from("current-creator"));
        whoami_mock.assert();
        current_issue_mock.assert();
        assert_eq!(whoami_mock.hits(), 1);
        assert_eq!(current_issue_mock.hits(), 1);
    }

    #[tokio::test]
    async fn update_issue_modifies_requested_fields() {
        let server = MockServer::start();
        let client = metis_client(&server);
        let target_issue_id = issue_id("i-9");
        let mut job_settings = JobSettings::default();
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
                "Updated issue description".into(),
                empty_user(),
                "New progress".into(),
                IssueStatus::Closed,
                Some("owner-b".into()),
                Some(job_settings.clone()),
                Vec::new(),
                vec![IssueDependency::new(
                    IssueDependencyType::BlockedOn,
                    issue_id("i-2"),
                )],
                vec![patch_id("p-3")],
                false,
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
            Some(IssueStatus::Closed),
            Some("owner-b".into()),
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
        let client = metis_client(&server);
        let target_issue_id = issue_id("i-10");
        let current_issue = IssueVersionRecord::new(
            target_issue_id.clone(),
            0,
            Utc::now(),
            Issue::new(
                IssueType::Feature,
                "Existing issue".into(),
                empty_user(),
                "Started work".into(),
                IssueStatus::InProgress,
                Some("owner-a".into()),
                None,
                Vec::new(),
                vec![IssueDependency::new(
                    IssueDependencyType::BlockedOn,
                    issue_id("i-5"),
                )],
                Vec::new(),
                false,
            ),
        );
        let update_request = UpsertIssueRequest::new(
            Issue::new(
                IssueType::Feature,
                "Existing issue".into(),
                empty_user(),
                String::new(),
                IssueStatus::InProgress,
                None,
                None,
                Vec::new(),
                vec![],
                Vec::new(),
                false,
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
        let client = metis_client(&server);
        let target_issue_id = issue_id("i-11");
        let job_settings = sample_job_settings();
        let current_issue = IssueVersionRecord::new(
            target_issue_id.clone(),
            0,
            Utc::now(),
            Issue::new(
                IssueType::Feature,
                "Existing issue".into(),
                empty_user(),
                "Started work".into(),
                IssueStatus::InProgress,
                Some("owner-a".into()),
                Some(job_settings),
                Vec::new(),
                vec![IssueDependency::new(
                    IssueDependencyType::BlockedOn,
                    issue_id("i-5"),
                )],
                Vec::new(),
                false,
            ),
        );
        let update_request = UpsertIssueRequest::new(
            Issue::new(
                IssueType::Feature,
                "Existing issue".into(),
                empty_user(),
                "Started work".into(),
                IssueStatus::InProgress,
                Some("owner-a".into()),
                None,
                Vec::new(),
                vec![IssueDependency::new(
                    IssueDependencyType::BlockedOn,
                    issue_id("i-5"),
                )],
                Vec::new(),
                false,
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
        let client = metis_client(&server);
        let target_issue_id = issue_id("i-secrets");
        let current_issue = IssueVersionRecord::new(
            target_issue_id.clone(),
            0,
            Utc::now(),
            Issue::new(
                IssueType::Task,
                "Existing issue".into(),
                empty_user(),
                String::new(),
                IssueStatus::Open,
                None,
                None,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                false,
            ),
        );
        let mut expected_settings = JobSettings::default();
        expected_settings.secrets = Some(vec!["new-secret".into()]);
        let update_request = UpsertIssueRequest::new(
            Issue::new(
                IssueType::Task,
                "Existing issue".into(),
                empty_user(),
                String::new(),
                IssueStatus::Open,
                None,
                Some(expected_settings),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                false,
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
        let client = metis_client(&server);
        let target_issue_id = issue_id("i-clear-secrets");
        let mut existing_settings = JobSettings::default();
        existing_settings.secrets = Some(vec!["old-secret".into()]);
        let current_issue = IssueVersionRecord::new(
            target_issue_id.clone(),
            0,
            Utc::now(),
            Issue::new(
                IssueType::Task,
                "Existing issue".into(),
                empty_user(),
                String::new(),
                IssueStatus::Open,
                None,
                Some(existing_settings),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                false,
            ),
        );
        let update_request = UpsertIssueRequest::new(
            Issue::new(
                IssueType::Task,
                "Existing issue".into(),
                empty_user(),
                String::new(),
                IssueStatus::Open,
                None,
                Some(JobSettings::default()),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                false,
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
                    "First issue\nwith context".into(),
                    empty_user(),
                    "Working on repro".into(),
                    IssueStatus::Open,
                    Some("owner-a".into()),
                    None,
                    Vec::new(),
                    vec![IssueDependency::new(
                        IssueDependencyType::BlockedOn,
                        issue_id("i-99"),
                    )],
                    Vec::new(),
                    false,
                ),
            ),
            IssueVersionRecord::new(
                issue_id("i-2"),
                0,
                Utc::now(),
                Issue::new(
                    IssueType::Feature,
                    "Follow-up work".into(),
                    empty_user(),
                    String::new(),
                    IssueStatus::InProgress,
                    None,
                    None,
                    Vec::new(),
                    vec![],
                    Vec::new(),
                    false,
                ),
            ),
        ];

        let mut output = Vec::new();
        render_issue_records(ResolvedOutputFormat::Pretty, &issues, &mut output).unwrap();
        let rendered = String::from_utf8(output).unwrap();
        let first_issue = issue_id("i-1").to_string();
        let dependency_id = issue_id("i-99").to_string();
        let second_issue = issue_id("i-2").to_string();

        assert!(rendered.contains(&format!("Issue {first_issue} (bug, open)")));
        assert!(rendered.contains("Assignee: owner-a"));
        assert!(rendered.contains("Description:\n  First issue\n  with context"));
        assert!(rendered.contains("Progress:\n  Working on repro"));
        assert!(rendered.contains(&format!("Dependencies:\n  - blocked-on {dependency_id}")));
        assert!(rendered.contains(&format!("Issue {second_issue} (feature, in-progress)")));
        assert!(rendered.contains("Assignee: -"));
        assert!(rendered.contains("Progress:\n  -"));
        assert!(rendered.contains("Dependencies: none"));
        assert!(rendered.contains("Follow-up work"));
    }

    #[tokio::test]
    async fn todo_command_fetches_existing_list() {
        let server = MockServer::start();
        let client = metis_client(&server);
        let issue_id = issue_id("i-todo");
        let todo_list = vec![
            TodoItem::new("write docs".into(), false),
            TodoItem::new("add tests".into(), true),
        ];
        let get_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{issue_id}").as_str());
            then.status(200).json_body_obj(&IssueVersionRecord::new(
                issue_id.clone(),
                0,
                Utc::now(),
                Issue::new(
                    IssueType::Task,
                    "has todos".into(),
                    empty_user(),
                    String::new(),
                    IssueStatus::Open,
                    None,
                    None,
                    todo_list.clone(),
                    vec![],
                    Vec::new(),
                    false,
                ),
            ));
        });

        let resolved = resolve_todo_list(&client, &issue_id, None, None, None, None)
            .await
            .unwrap();

        get_mock.assert();
        assert_eq!(get_mock.hits(), 1);
        assert_eq!(resolved, todo_list);
    }

    #[tokio::test]
    async fn todo_command_adds_item_and_parses_done_prefix() {
        let server = MockServer::start();
        let client = metis_client(&server);
        let issue_id = issue_id("i-add");
        let add_request = AddTodoItemRequest::new("finish docs".into(), true);
        let add_mock = server.mock(|when, then| {
            when.method(POST)
                .path(format!("/v1/issues/{issue_id}/todo-items").as_str())
                .json_body_obj(&add_request);
            then.status(200)
                .json_body_obj(&TodoListResponse::new(issue_id.clone(), Vec::new()));
        });

        let updated = resolve_todo_list(
            &client,
            &issue_id,
            Some("[x] finish docs".into()),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        add_mock.assert();
        assert_eq!(add_mock.hits(), 1);
        assert!(updated.is_empty());
    }

    #[tokio::test]
    async fn todo_command_marks_item_done_and_undone() {
        let server = MockServer::start();
        let client = metis_client(&server);
        let issue_id = issue_id("i-done");
        let mark_done_request = SetTodoItemStatusRequest::new(true);
        let mark_done_mock = server.mock(|when, then| {
            when.method(POST)
                .path(format!("/v1/issues/{issue_id}/todo-items/1").as_str())
                .json_body_obj(&mark_done_request);
            then.status(200).json_body_obj(&TodoListResponse::new(
                issue_id.clone(),
                vec![TodoItem::new("first".into(), true)],
            ));
        });

        let done_list = resolve_todo_list(&client, &issue_id, None, Some(1), None, None)
            .await
            .unwrap();
        mark_done_mock.assert();
        assert_eq!(mark_done_mock.hits(), 1);
        assert!(done_list[0].is_done);

        let mark_undone_request = SetTodoItemStatusRequest::new(false);
        let mark_undone_mock = server.mock(|when, then| {
            when.method(POST)
                .path(format!("/v1/issues/{issue_id}/todo-items/1").as_str())
                .json_body_obj(&mark_undone_request);
            then.status(200).json_body_obj(&TodoListResponse::new(
                issue_id.clone(),
                vec![TodoItem::new("first".into(), false)],
            ));
        });

        let undone_list = resolve_todo_list(&client, &issue_id, None, None, Some(1), None)
            .await
            .unwrap();
        mark_undone_mock.assert();
        assert_eq!(mark_undone_mock.hits(), 1);
        assert!(!undone_list[0].is_done);
    }

    #[tokio::test]
    async fn todo_command_replaces_list_with_parsed_items() {
        let server = MockServer::start();
        let client = metis_client(&server);
        let issue_id = issue_id("i-replace");
        let parsed = vec![
            TodoItem::new("first item".into(), false),
            TodoItem::new("second".into(), true),
        ];
        let replace_mock = server.mock(|when, then| {
            when.method(PUT)
                .path(format!("/v1/issues/{issue_id}/todo-items").as_str())
                .json_body_obj(&ReplaceTodoListRequest::new(parsed.clone()));
            then.status(200)
                .json_body_obj(&TodoListResponse::new(issue_id.clone(), parsed.clone()));
        });

        let resolved = resolve_todo_list(
            &client,
            &issue_id,
            None,
            None,
            None,
            Some(vec!["first item".into(), "[x] second".into()]),
        )
        .await
        .unwrap();

        replace_mock.assert();
        assert_eq!(replace_mock.hits(), 1);
        assert_eq!(resolved, parsed);
    }

    #[test]
    fn render_todo_list_formats_output() {
        let todo_list = vec![
            TodoItem::new("write docs".into(), false),
            TodoItem::new("add tests\nwith details".into(), true),
        ];
        let mut output = Vec::new();
        render_todo_list(
            ResolvedOutputFormat::Pretty,
            &issue_id("i-render"),
            &todo_list,
            &mut output,
        )
        .unwrap();
        let rendered = String::from_utf8(output).unwrap();

        assert!(rendered.contains("Todos for issue i-render:"));
        assert!(rendered.contains("1. [ ] write docs"));
        assert!(rendered.contains("2. [x] add tests"));
        assert!(
            rendered.contains("         with details"),
            "continuation lines should be indented"
        );
    }

    #[test]
    fn describe_issue_pretty_printer_includes_sections() {
        let main_patch_id = patch_id("p-main");
        let main_patch_record = PatchVersionRecord::new(
            main_patch_id.clone(),
            0,
            Utc::now(),
            Patch::new(
                "main patch".into(),
                "desc".into(),
                sample_diff(),
                Default::default(),
                false,
                None,
                Vec::new(),
                sample_repo_name(),
                None,
                false,
            ),
        );
        let description = IssueDescription {
            issue: IssueWithPatches {
                issue: IssueVersionRecord::new(
                    issue_id("i-main"),
                    0,
                    Utc::now(),
                    Issue::new(
                        IssueType::Task,
                        "Main issue".into(),
                        empty_user(),
                        String::new(),
                        IssueStatus::Open,
                        Some("owner".into()),
                        None,
                        Vec::new(),
                        vec![],
                        vec![main_patch_id],
                        false,
                    ),
                ),
                patches: vec![main_patch_record],
            },
            parents: vec![IssueWithPatches {
                issue: IssueVersionRecord::new(
                    issue_id("i-parent"),
                    0,
                    Utc::now(),
                    Issue::new(
                        IssueType::Feature,
                        "Parent".into(),
                        empty_user(),
                        String::new(),
                        IssueStatus::Open,
                        None,
                        None,
                        Vec::new(),
                        vec![],
                        Vec::new(),
                        false,
                    ),
                ),
                patches: Vec::new(),
            }],
            children: vec![],
            activity_log: Vec::new(),
        };

        let mut output = Vec::new();
        let summary = summarize_issue_description(&description).unwrap();
        print_issue_description_pretty(&summary, &mut output).unwrap();
        let rendered = strip_ansi_codes(&String::from_utf8(output).unwrap());

        assert!(rendered.contains("Issue"));
        assert!(rendered.contains("Parents:"));
        assert!(rendered.contains("Children (transitive):"));
        assert!(rendered.contains("i-parent"));
        assert!(rendered.contains("main patch (p-main) [open]"));
        assert!(rendered.contains("      Description:\n        desc"));
        assert!(rendered.contains("Reviews: none"));
    }

    #[test]
    fn describe_issue_pretty_printer_includes_history() {
        let main_issue_id = issue_id("i-main");
        let main_patch_id = patch_id("p-main");
        let main_job_id = task_id("t-main");

        let base_issue = Issue::new(
            IssueType::Task,
            "Main issue".into(),
            empty_user(),
            String::new(),
            IssueStatus::Open,
            Some("owner".into()),
            None,
            Vec::new(),
            vec![],
            vec![main_patch_id.clone()],
            false,
        );
        let mut updated_issue = base_issue.clone();
        updated_issue.status = IssueStatus::InProgress;

        let issue_versions = vec![
            Versioned::new(
                base_issue,
                1,
                Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap(),
            ),
            Versioned::new(
                updated_issue,
                2,
                Utc.with_ymd_and_hms(2024, 1, 4, 9, 0, 0).unwrap(),
            ),
        ];
        let mut activity_log =
            activity_log_for_issue_versions(main_issue_id.clone(), &issue_versions);

        let base_patch = Patch::new(
            "main patch".into(),
            "desc".into(),
            sample_diff(),
            PatchStatus::Open,
            false,
            None,
            Vec::new(),
            sample_repo_name(),
            None,
            false,
        );
        let mut updated_patch = base_patch.clone();
        updated_patch.status = PatchStatus::Merged;
        let patch_versions = vec![
            Versioned::new(
                base_patch,
                1,
                Utc.with_ymd_and_hms(2024, 1, 2, 9, 0, 0).unwrap(),
            ),
            Versioned::new(
                updated_patch,
                2,
                Utc.with_ymd_and_hms(2024, 1, 3, 9, 0, 0).unwrap(),
            ),
        ];
        activity_log.extend(activity_log_for_patch_versions(
            main_patch_id.clone(),
            &patch_versions,
        ));

        let base_task = Task::new_with_status(
            "run build".into(),
            BundleSpec::None,
            Some(main_issue_id.clone()),
            None,
            None,
            HashMap::new(),
            None,
            None,
            None,
            Status::Created,
            None,
            None,
            false,
        );
        let mut updated_task = base_task.clone();
        updated_task.status = Status::Running;
        let job_versions = vec![
            Versioned::new(
                base_task,
                1,
                Utc.with_ymd_and_hms(2024, 1, 2, 12, 0, 0).unwrap(),
            ),
            Versioned::new(
                updated_task,
                2,
                Utc.with_ymd_and_hms(2024, 1, 2, 15, 0, 0).unwrap(),
            ),
        ];
        activity_log.extend(activity_log_for_job_versions(
            main_job_id.clone(),
            &job_versions,
        ));
        sort_activity_log_entries(&mut activity_log);

        let description = IssueDescription {
            issue: IssueWithPatches {
                issue: IssueVersionRecord::new(
                    main_issue_id.clone(),
                    0,
                    Utc::now(),
                    issue_versions[1].item.clone(),
                ),
                patches: Vec::new(),
            },
            parents: vec![],
            children: vec![],
            activity_log,
        };

        let mut output = Vec::new();
        let summary = summarize_issue_description(&description).unwrap();
        print_issue_description_pretty(&summary, &mut output).unwrap();
        let rendered = strip_ansi_codes(&String::from_utf8(output).unwrap());

        assert!(rendered.contains("History:"));
        assert!(rendered.contains("2024-01-01T12:00:00Z Issue i-main v1 created"));
        assert!(rendered.contains("2024-01-02T09:00:00Z Patch p-main v1 created"));
        assert!(rendered.contains("2024-01-02T12:00:00Z Job t-main v1 created"));
        assert!(rendered.contains("Status: open -> in-progress"));
    }

    #[test]
    fn describe_issue_pretty_printer_includes_progress() {
        let description = IssueDescription {
            issue: IssueWithPatches {
                issue: IssueVersionRecord::new(
                    issue_id("i-main"),
                    0,
                    Utc::now(),
                    Issue::new(
                        IssueType::Task,
                        "Main issue".into(),
                        empty_user(),
                        "Main progress".into(),
                        IssueStatus::Open,
                        Some("owner".into()),
                        None,
                        Vec::new(),
                        vec![],
                        Vec::new(),
                        false,
                    ),
                ),
                patches: Vec::new(),
            },
            parents: vec![IssueWithPatches {
                issue: IssueVersionRecord::new(
                    issue_id("i-parent"),
                    0,
                    Utc::now(),
                    Issue::new(
                        IssueType::Feature,
                        "Parent".into(),
                        empty_user(),
                        String::new(),
                        IssueStatus::Open,
                        None,
                        None,
                        Vec::new(),
                        vec![],
                        Vec::new(),
                        false,
                    ),
                ),
                patches: Vec::new(),
            }],
            children: vec![IssueWithPatches {
                issue: IssueVersionRecord::new(
                    issue_id("i-child"),
                    0,
                    Utc::now(),
                    Issue::new(
                        IssueType::Bug,
                        "Child".into(),
                        empty_user(),
                        "Child update".into(),
                        IssueStatus::InProgress,
                        None,
                        None,
                        Vec::new(),
                        vec![],
                        Vec::new(),
                        false,
                    ),
                ),
                patches: Vec::new(),
            }],
            activity_log: Vec::new(),
        };

        let mut output = Vec::new();
        let summary = summarize_issue_description(&description).unwrap();
        print_issue_description_pretty(&summary, &mut output).unwrap();
        let rendered = strip_ansi_codes(&String::from_utf8(output).unwrap());

        assert!(rendered.contains("Progress:\n    Main progress"));
        assert!(rendered.contains("Parents:\n  i-parent"));
        assert!(rendered.contains("Children (transitive):\n  i-child"));
    }

    #[test]
    fn describe_issue_pretty_printer_shows_todos_for_root_issue_only() {
        let root_todos = vec![
            TodoItem::new("root todo".into(), false),
            TodoItem::new("root done".into(), true),
        ];
        let description = IssueDescription {
            issue: IssueWithPatches {
                issue: IssueVersionRecord::new(
                    issue_id("i-main"),
                    0,
                    Utc::now(),
                    Issue::new(
                        IssueType::Task,
                        "Main issue".into(),
                        empty_user(),
                        String::new(),
                        IssueStatus::Open,
                        Some("owner".into()),
                        None,
                        root_todos.clone(),
                        vec![],
                        Vec::new(),
                        false,
                    ),
                ),
                patches: Vec::new(),
            },
            parents: vec![IssueWithPatches {
                issue: IssueVersionRecord::new(
                    issue_id("i-parent"),
                    0,
                    Utc::now(),
                    Issue::new(
                        IssueType::Task,
                        "Parent description".into(),
                        empty_user(),
                        String::new(),
                        IssueStatus::Open,
                        None,
                        None,
                        vec![TodoItem::new("parent todo".into(), false)],
                        vec![],
                        Vec::new(),
                        false,
                    ),
                ),
                patches: Vec::new(),
            }],
            children: vec![IssueWithPatches {
                issue: IssueVersionRecord::new(
                    issue_id("i-child"),
                    0,
                    Utc::now(),
                    Issue::new(
                        IssueType::Bug,
                        "Child description".into(),
                        empty_user(),
                        String::new(),
                        IssueStatus::Open,
                        None,
                        None,
                        vec![TodoItem::new("child todo".into(), true)],
                        vec![],
                        Vec::new(),
                        false,
                    ),
                ),
                patches: Vec::new(),
            }],
            activity_log: Vec::new(),
        };

        let mut output = Vec::new();
        let summary = summarize_issue_description(&description).unwrap();
        print_issue_description_pretty(&summary, &mut output).unwrap();
        let rendered = strip_ansi_codes(&String::from_utf8(output).unwrap());

        assert!(rendered.contains("Todos:\n    1. [ ] root todo\n    2. [x] root done"));
        assert!(!rendered.contains("parent todo"));
        assert!(!rendered.contains("child todo"));
    }

    #[test]
    fn describe_issue_pretty_printer_shows_review_summary() {
        let main_patch_id = patch_id("p-main");
        let earliest_review = Utc.with_ymd_and_hms(2024, 5, 1, 11, 50, 0).unwrap();
        let latest_review = earliest_review + Duration::minutes(10);
        let patch_reviews = vec![
            Review::new(
                "needs work".to_string(),
                false,
                "alex".to_string(),
                Some(earliest_review),
            ),
            Review::new(
                "fixed now".to_string(),
                false,
                "sam".to_string(),
                Some(earliest_review + Duration::minutes(5)),
            ),
            Review::new(
                "ship it".to_string(),
                true,
                "sam".to_string(),
                Some(latest_review),
            ),
        ];
        let description = IssueDescription {
            issue: IssueWithPatches {
                issue: IssueVersionRecord::new(
                    issue_id("i-main"),
                    0,
                    Utc::now(),
                    Issue::new(
                        IssueType::Task,
                        "Main issue".into(),
                        empty_user(),
                        String::new(),
                        IssueStatus::Open,
                        Some("owner".into()),
                        None,
                        Vec::new(),
                        vec![],
                        vec![main_patch_id.clone()],
                        false,
                    ),
                ),
                patches: vec![PatchVersionRecord::new(
                    main_patch_id,
                    0,
                    Utc::now(),
                    Patch::new(
                        "main patch".into(),
                        "desc".into(),
                        sample_diff(),
                        Default::default(),
                        false,
                        None,
                        patch_reviews,
                        sample_repo_name(),
                        None,
                        false,
                    ),
                )],
            },
            parents: vec![],
            children: vec![],
            activity_log: Vec::new(),
        };

        let mut output = Vec::new();
        let summary = summarize_issue_description(&description).unwrap();
        print_issue_description_pretty(&summary, &mut output).unwrap();
        let rendered = strip_ansi_codes(&String::from_utf8(output).unwrap());

        assert!(
            rendered.contains("Latest: approved by sam @ 2024-05-01T12:00:00Z"),
            "latest review should reflect the newest review"
        );
        assert!(rendered.contains("Counts: 1 approval, 2 change requests"));
        assert!(rendered.contains("Reviewers:"));
        assert!(rendered.contains("- alex: changes requested @ 2024-05-01T11:50:00Z"));
        assert!(rendered.contains("- sam: approved @ 2024-05-01T12:00:00Z"));
    }
}
