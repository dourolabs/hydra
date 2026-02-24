use anyhow::{Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use metis_common::{
    issues::{Issue, IssueStatus, IssueType},
    jobs::Task,
    patches::{PatchStatus, Review},
    task_status::Status,
    ActivityEvent, ActivityLogEntry, ActivityObjectKind, FieldChange, MetisId, RepoName, TaskId,
    VersionNumber,
};
use owo_colors::OwoColorize;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    io::Write,
};

// ---------------------------------------------------------------------------
// Summary types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ActivityLogEntrySummary {
    pub object_id: MetisId,
    pub object_kind: ActivityObjectKind,
    pub version: VersionNumber,
    pub timestamp: DateTime<Utc>,
    pub event: ActivityEventSummary,
    pub object: ActivityObjectSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<metis_common::actor_ref::ActorRef>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActivityEventSummary {
    Created,
    Updated {
        changes: Vec<ActivityFieldChangeSummary>,
        #[serde(skip_serializing_if = "Vec::is_empty", default)]
        other_changes: Vec<String>,
    },
}

#[derive(Debug, Serialize)]
pub struct ActivityFieldChangeSummary {
    pub field: String,
    pub before: Value,
    pub after: Value,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActivityObjectSummary {
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
    Document {
        title: String,
        path: Option<String>,
        body_markdown: String,
    },
}

#[derive(Debug, Serialize)]
pub struct ReviewSummary {
    pub contents: String,
    pub is_approved: bool,
    pub author: String,
}

// ---------------------------------------------------------------------------
// Summarization functions
// ---------------------------------------------------------------------------

pub fn summarize_activity_log(
    entries: &[ActivityLogEntry],
) -> Result<Vec<ActivityLogEntrySummary>> {
    entries.iter().map(summarize_activity_log_entry).collect()
}

pub fn summarize_activity_log_entry(entry: &ActivityLogEntry) -> Result<ActivityLogEntrySummary> {
    let object = summarize_activity_object(entry)?;
    let event = summarize_activity_event(entry, &object)?;

    Ok(ActivityLogEntrySummary {
        object_id: entry.object_id.clone(),
        object_kind: entry.object_kind.clone(),
        version: entry.version,
        timestamp: entry.timestamp,
        event,
        object,
        actor: entry.actor.clone(),
    })
}

pub fn summarize_activity_object(entry: &ActivityLogEntry) -> Result<ActivityObjectSummary> {
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
        ActivityObjectKind::Document => {
            let doc: metis_common::api::v1::documents::Document = decode_activity_object(entry)?;
            Ok(ActivityObjectSummary::Document {
                title: doc.title,
                path: doc.path.map(|p| p.to_string()),
                body_markdown: doc.body_markdown,
            })
        }
        _ => Ok(ActivityObjectSummary::Job {
            status: Status::Unknown,
        }),
    }
}

pub fn summarize_activity_event(
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

pub fn summarize_activity_changes(
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

// ---------------------------------------------------------------------------
// Field tracking
// ---------------------------------------------------------------------------

pub fn tracked_field_for_path(kind: &ActivityObjectKind, path: &str) -> Option<&'static str> {
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
        ActivityObjectKind::Document => match path {
            "/title" => Some("Title"),
            "/path" => Some("Path"),
            "/body_markdown" => Some("Body"),
            _ => None,
        },
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

pub fn reconstruct_before_object(entry: &ActivityLogEntry) -> Option<Value> {
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

pub fn summarize_reviews_value(reviews: Option<&[Review]>) -> Value {
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

fn decode_activity_object<T: DeserializeOwned>(entry: &ActivityLogEntry) -> Result<T> {
    serde_json::from_value(entry.object.clone()).context("failed to decode activity log object")
}

// ---------------------------------------------------------------------------
// Pretty-printing functions
// ---------------------------------------------------------------------------

pub fn write_activity_log_pretty(
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

pub fn write_activity_log_entry_pretty(
    entry: &ActivityLogEntrySummary,
    indent: &str,
    writer: &mut impl Write,
) -> Result<()> {
    let timestamp = entry.timestamp.to_rfc3339_opts(SecondsFormat::Secs, true);
    let kind_label = match entry.object_kind {
        ActivityObjectKind::Issue => "Issue",
        ActivityObjectKind::Patch => "Patch",
        ActivityObjectKind::Job => "Job",
        ActivityObjectKind::Document => "Document",
        _ => "Activity",
    };
    let event_label = match entry.event {
        ActivityEventSummary::Created => "created",
        ActivityEventSummary::Updated { .. } => "updated",
    };

    let actor_label = entry
        .actor
        .as_ref()
        .and_then(format_actor_label)
        .map(|label| format!(" by {label}"))
        .unwrap_or_default();

    writeln!(
        writer,
        "{indent}{} {} {} v{} {}{}",
        colorize_dimmed(&timestamp),
        colorize_bold(kind_label),
        entry.object_id,
        entry.version,
        event_label,
        actor_label
    )?;

    let detail_indent = format!("{indent}  ");
    write_activity_object_summary(&entry.object, &entry.event, &detail_indent, writer)?;

    Ok(())
}

fn format_actor_label(actor: &metis_common::actor_ref::ActorRef) -> Option<String> {
    Some(actor.display_name())
}

pub fn write_activity_object_summary(
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
        ActivityObjectSummary::Document {
            title,
            path,
            body_markdown,
        } => {
            write_activity_scalar_field(
                "Title",
                &Value::String(title.clone()),
                change_map.get("Title").copied(),
                indent,
                writer,
            )?;
            write_activity_optional_scalar_field(
                "Path",
                path.as_deref(),
                change_map.get("Path").copied(),
                indent,
                writer,
            )?;
            write_activity_multiline_field(
                "Body",
                body_markdown,
                change_map.get("Body").copied(),
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

pub fn write_activity_scalar_field(
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

pub fn write_activity_optional_scalar_field(
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

pub fn write_activity_multiline_field(
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

pub fn write_activity_reviews(
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

// ---------------------------------------------------------------------------
// Changelog-specific pretty-printing (simplified format for `changelog` cmd)
// ---------------------------------------------------------------------------

/// Write a changelog in the simplified `vN timestamp event by actor` format.
///
/// Short fields show before -> after values; large fields just show "changed".
pub fn write_changelog_pretty(
    entries: &[ActivityLogEntrySummary],
    writer: &mut impl Write,
) -> Result<()> {
    for (index, entry) in entries.iter().enumerate() {
        write_changelog_entry_pretty(entry, writer)?;
        if index + 1 < entries.len() {
            writeln!(writer)?;
        }
    }
    Ok(())
}

fn write_changelog_entry_pretty(
    entry: &ActivityLogEntrySummary,
    writer: &mut impl Write,
) -> Result<()> {
    let timestamp = entry.timestamp.to_rfc3339_opts(SecondsFormat::Secs, true);
    let event_label = match entry.event {
        ActivityEventSummary::Created => "created",
        ActivityEventSummary::Updated { .. } => "updated",
    };

    let actor_label = entry
        .actor
        .as_ref()
        .and_then(format_actor_label)
        .map(|label| format!(" by {label}"))
        .unwrap_or_default();

    writeln!(
        writer,
        "v{} {} {}{actor_label}",
        entry.version, timestamp, event_label,
    )?;

    let indent = "  ";
    write_changelog_fields(&entry.object, &entry.event, indent, writer)?;

    Ok(())
}

/// Large fields are printed as "changed" rather than showing the full value.
const LARGE_FIELDS: &[&str] = &["Description", "Progress", "Body"];

fn is_large_field(label: &str) -> bool {
    LARGE_FIELDS.contains(&label)
}

fn write_changelog_fields(
    object: &ActivityObjectSummary,
    event: &ActivityEventSummary,
    indent: &str,
    writer: &mut impl Write,
) -> Result<()> {
    match event {
        ActivityEventSummary::Created => {
            write_changelog_created_fields(object, indent, writer)?;
        }
        ActivityEventSummary::Updated {
            changes,
            other_changes,
        } => {
            for change in changes {
                if is_large_field(&change.field) {
                    writeln!(writer, "{indent}{}: changed", change.field)?;
                } else {
                    writeln!(
                        writer,
                        "{indent}{}: {} -> {}",
                        change.field,
                        format_changelog_value(&change.before),
                        format_changelog_value(&change.after),
                    )?;
                }
            }
            if !other_changes.is_empty() {
                let joined = other_changes.join(", ");
                writeln!(writer, "{indent}Other changes: {joined}")?;
            }
        }
    }
    Ok(())
}

fn write_changelog_created_fields(
    object: &ActivityObjectSummary,
    indent: &str,
    writer: &mut impl Write,
) -> Result<()> {
    match object {
        ActivityObjectSummary::Issue {
            issue_type, status, ..
        } => {
            writeln!(writer, "{indent}Type: {issue_type}")?;
            writeln!(writer, "{indent}Status: {status}")?;
        }
        ActivityObjectSummary::Patch {
            title,
            status,
            repo,
            ..
        } => {
            writeln!(writer, "{indent}Title: {title}")?;
            writeln!(writer, "{indent}Status: {status}")?;
            writeln!(writer, "{indent}Repo: {repo}")?;
        }
        ActivityObjectSummary::Job { status } => {
            writeln!(writer, "{indent}Status: {}", format_job_status(*status))?;
        }
        ActivityObjectSummary::Document { title, path, .. } => {
            writeln!(writer, "{indent}Title: {title}")?;
            if let Some(p) = path {
                writeln!(writer, "{indent}Path: {p}")?;
            }
        }
    }
    Ok(())
}

fn format_changelog_value(value: &Value) -> String {
    match value {
        Value::String(s) => {
            // Normalize status fields: replace underscores with hyphens
            let display = s.replace('_', "-");
            format!("\"{display}\"")
        }
        Value::Null => "(none)".to_string(),
        _ => format_activity_value(value),
    }
}

// ---------------------------------------------------------------------------
// Formatting helpers (shared)
// ---------------------------------------------------------------------------

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

pub fn format_job_status(status: Status) -> &'static str {
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
