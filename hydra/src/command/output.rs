use std::io::Write;

use anyhow::Result;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use clap::ValueEnum;
use hydra_common::{
    agents::AgentRecord,
    api::v1::{
        messages::VersionedMessage,
        notifications::{
            ListNotificationsResponse, MarkReadResponse, NotificationResponse, UnreadCountResponse,
        },
    },
    documents::{DocumentSummaryRecord, DocumentVersionRecord},
    issues::{Issue, IssueSummary, IssueSummaryRecord, IssueVersionRecord},
    patches::{PatchStatus, PatchSummaryRecord, PatchVersionRecord},
    repositories::RepositoryRecord,
    sessions::{Session, SessionSummary, SessionSummaryRecord, SessionVersionRecord},
    task_status::{Status, TaskError},
    whoami::ActorIdentity,
    NotificationId,
};
use owo_colors::OwoColorize;
use textwrap::{termwidth, Options, WrapAlgorithm};

use crate::client::HydraClientInterface;
use crate::util::{format_duration, truncate_lines};

const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const RESET: &str = "\x1b[0m";
const NAME_WIDTH: usize = 48;
const STATUS_WIDTH: usize = 26;
const RUNTIME_WIDTH: usize = 12;
const MAX_NOTES_WIDTH: usize = 80;
const MAX_NOTE_LINES: usize = 5;
const MAX_DOCUMENT_BODY_LINES: usize = 20;
const MAX_DOCUMENT_BODY_WIDTH: usize = 120;
const DEFAULT_TERMINAL_WIDTH: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Auto,
    Jsonl,
    Pretty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedOutputFormat {
    Jsonl,
    Pretty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandContext {
    pub output_format: ResolvedOutputFormat,
}

impl CommandContext {
    pub fn new(output_format: ResolvedOutputFormat) -> Self {
        Self { output_format }
    }
}

pub async fn resolve_output_format(
    client: &dyn HydraClientInterface,
    output_format: OutputFormat,
) -> Result<ResolvedOutputFormat> {
    match output_format {
        OutputFormat::Auto => resolve_auto_output_format(client).await,
        OutputFormat::Jsonl => Ok(ResolvedOutputFormat::Jsonl),
        OutputFormat::Pretty => Ok(ResolvedOutputFormat::Pretty),
    }
}

pub fn render_issue_records(
    format: ResolvedOutputFormat,
    issues: &[IssueVersionRecord],
    writer: &mut impl Write,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => render_issue_records_jsonl(issues, writer),
        ResolvedOutputFormat::Pretty => render_issue_records_pretty(issues, writer),
    }
}

pub fn render_issue_summary_records(
    format: ResolvedOutputFormat,
    issues: &[IssueSummaryRecord],
    writer: &mut impl Write,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => render_issue_summary_records_jsonl(issues, writer),
        ResolvedOutputFormat::Pretty => render_issue_summary_records_pretty(issues, writer),
    }
}

pub fn render_patch_records(
    format: ResolvedOutputFormat,
    patches: &[PatchVersionRecord],
    writer: &mut impl Write,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => render_patch_records_jsonl(patches, writer),
        ResolvedOutputFormat::Pretty => render_patch_records_pretty(patches, writer),
    }
}

pub fn render_patch_summary_records(
    format: ResolvedOutputFormat,
    patches: &[PatchSummaryRecord],
    writer: &mut impl Write,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => render_patch_summary_records_jsonl(patches, writer),
        ResolvedOutputFormat::Pretty => render_patch_summary_records_pretty(patches, writer),
    }
}

pub fn render_session_records(
    format: ResolvedOutputFormat,
    jobs: &[SessionVersionRecord],
    writer: &mut impl Write,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => render_session_records_jsonl(jobs, writer),
        ResolvedOutputFormat::Pretty => render_session_records_pretty(jobs, writer),
    }
}

pub fn render_session_summary_records(
    format: ResolvedOutputFormat,
    jobs: &[SessionSummaryRecord],
    writer: &mut impl Write,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => render_session_summary_records_jsonl(jobs, writer),
        ResolvedOutputFormat::Pretty => render_session_summary_records_pretty(jobs, writer),
    }
}

pub fn render_agent_records(
    format: ResolvedOutputFormat,
    agents: &[AgentRecord],
    writer: &mut impl Write,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => render_agent_records_jsonl(agents, writer),
        ResolvedOutputFormat::Pretty => render_agent_records_pretty(agents, writer),
    }
}

pub fn render_repository_records(
    format: ResolvedOutputFormat,
    repositories: &[RepositoryRecord],
    writer: &mut impl Write,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => render_repository_records_jsonl(repositories, writer),
        ResolvedOutputFormat::Pretty => render_repository_records_pretty(repositories, writer),
    }
}

pub fn render_document_records(
    format: ResolvedOutputFormat,
    documents: &[DocumentVersionRecord],
    full_output: bool,
    writer: &mut impl Write,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => render_document_records_jsonl(documents, writer),
        ResolvedOutputFormat::Pretty => {
            render_document_records_pretty(documents, full_output, writer)
        }
    }
}

pub fn render_document_summary_records(
    format: ResolvedOutputFormat,
    documents: &[DocumentSummaryRecord],
    writer: &mut impl Write,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => render_document_summary_records_jsonl(documents, writer),
        ResolvedOutputFormat::Pretty => render_document_summary_records_pretty(documents, writer),
    }
}

pub fn render_versioned_messages(
    format: ResolvedOutputFormat,
    messages: &[VersionedMessage],
    writer: &mut impl Write,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => render_versioned_messages_jsonl(messages, writer),
        ResolvedOutputFormat::Pretty => render_versioned_messages_pretty(messages, writer),
    }
}

async fn resolve_auto_output_format(
    client: &dyn HydraClientInterface,
) -> Result<ResolvedOutputFormat> {
    let whoami = client.whoami().await?;
    Ok(match whoami.actor {
        ActorIdentity::User { .. } => ResolvedOutputFormat::Pretty,
        ActorIdentity::Session { .. } => ResolvedOutputFormat::Jsonl,
        _ => ResolvedOutputFormat::Jsonl,
    })
}

fn render_issue_records_jsonl(
    issues: &[IssueVersionRecord],
    writer: &mut impl Write,
) -> Result<()> {
    for issue in issues {
        serde_json::to_writer(&mut *writer, issue)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn render_issue_records_pretty(
    issues: &[IssueVersionRecord],
    writer: &mut impl Write,
) -> Result<()> {
    for (index, issue_record) in issues.iter().enumerate() {
        let Issue {
            issue_type,
            title,
            description,
            creator,
            progress,
            status,
            assignee,
            dependencies,
            ..
        } = &issue_record.issue;

        writeln!(
            writer,
            "Issue {} ({issue_type}, {status})",
            issue_record.issue_id
        )?;
        if !title.is_empty() {
            writeln!(writer, "Title: {title}")?;
        }
        writeln!(writer, "Creator: {}", creator.as_ref())?;
        writeln!(writer, "Assignee: {}", assignee.as_deref().unwrap_or("-"))?;
        writeln!(writer, "Description:")?;
        if description.trim().is_empty() {
            writeln!(writer, "  -")?;
        } else {
            for line in description.lines() {
                writeln!(writer, "  {line}")?;
            }
        }

        writeln!(writer, "Progress:")?;
        if progress.trim().is_empty() {
            writeln!(writer, "  -")?;
        } else {
            for line in progress.lines() {
                writeln!(writer, "  {line}")?;
            }
        }

        if dependencies.is_empty() {
            writeln!(writer, "Dependencies: none")?;
        } else {
            writeln!(writer, "Dependencies:")?;
            for dependency in dependencies {
                writeln!(
                    writer,
                    "  - {} {}",
                    dependency.dependency_type, dependency.issue_id
                )?;
            }
        }

        if index + 1 < issues.len() {
            writeln!(writer)?;
        }
    }
    writer.flush()?;
    Ok(())
}

fn render_issue_summary_records_jsonl(
    issues: &[IssueSummaryRecord],
    writer: &mut impl Write,
) -> Result<()> {
    for issue in issues {
        serde_json::to_writer(&mut *writer, issue)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn render_issue_summary_records_pretty(
    issues: &[IssueSummaryRecord],
    writer: &mut impl Write,
) -> Result<()> {
    for (index, issue_record) in issues.iter().enumerate() {
        let IssueSummary {
            issue_type,
            title,
            description,
            creator,
            status,
            assignee,
            dependencies,
            ..
        } = &issue_record.issue;

        writeln!(
            writer,
            "Issue {} ({issue_type}, {status})",
            issue_record.issue_id
        )?;
        if !title.is_empty() {
            writeln!(writer, "Title: {title}")?;
        }
        writeln!(writer, "Creator: {}", creator.as_ref())?;
        writeln!(writer, "Assignee: {}", assignee.as_deref().unwrap_or("-"))?;
        writeln!(writer, "Description:")?;
        if description.trim().is_empty() {
            writeln!(writer, "  -")?;
        } else {
            for line in description.lines() {
                writeln!(writer, "  {line}")?;
            }
        }

        if dependencies.is_empty() {
            writeln!(writer, "Dependencies: none")?;
        } else {
            writeln!(writer, "Dependencies:")?;
            for dependency in dependencies {
                writeln!(
                    writer,
                    "  - {} {}",
                    dependency.dependency_type, dependency.issue_id
                )?;
            }
        }

        if index + 1 < issues.len() {
            writeln!(writer)?;
        }
    }
    writer.flush()?;
    Ok(())
}

fn render_patch_records_jsonl(
    patches: &[PatchVersionRecord],
    writer: &mut impl Write,
) -> Result<()> {
    for patch in patches {
        serde_json::to_writer(&mut *writer, patch)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn render_patch_records_pretty(
    patches: &[PatchVersionRecord],
    writer: &mut impl Write,
) -> Result<()> {
    for patch in patches {
        write_patch_record_pretty(patch, writer)?;
    }
    writer.flush()?;
    Ok(())
}

fn write_patch_record_pretty(record: &PatchVersionRecord, writer: &mut impl Write) -> Result<()> {
    let title = extract_patch_title(record);
    let status = extract_patch_status(record);
    let description = extract_patch_description(record);
    writeln!(
        writer,
        "Patch {} [{}]: {}",
        record.patch_id,
        format_patch_status(status),
        title
    )?;
    writeln!(
        writer,
        "Repository: {}",
        record.patch.service_repo_name.as_str()
    )?;
    if !description.trim().is_empty() {
        writeln!(writer, "{description}")?;
    }
    if record.patch.diff.trim().is_empty() {
        writeln!(writer, "[no diff available]")?;
    } else {
        writeln!(writer)?;
        pretty_print_patch(&record.patch.diff, writer)?;
    }
    writeln!(writer)?;
    Ok(())
}

fn pretty_print_patch(patch: &str, writer: &mut impl Write) -> Result<()> {
    for line in patch.lines() {
        if line.starts_with('+') && !line.starts_with("+++") {
            writeln!(writer, "{GREEN}{line}{RESET}")?;
        } else if line.starts_with('-') && !line.starts_with("---") {
            writeln!(writer, "{RED}{line}{RESET}")?;
        } else {
            writeln!(writer, "{line}")?;
        }
    }
    Ok(())
}

fn extract_patch_title(record: &PatchVersionRecord) -> &str {
    record.patch.title.as_str()
}

fn extract_patch_status(record: &PatchVersionRecord) -> PatchStatus {
    record.patch.status
}

fn extract_patch_description(record: &PatchVersionRecord) -> &str {
    record.patch.description.as_str()
}

fn render_patch_summary_records_jsonl(
    patches: &[PatchSummaryRecord],
    writer: &mut impl Write,
) -> Result<()> {
    for patch in patches {
        serde_json::to_writer(&mut *writer, patch)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn render_patch_summary_records_pretty(
    patches: &[PatchSummaryRecord],
    writer: &mut impl Write,
) -> Result<()> {
    for record in patches {
        writeln!(
            writer,
            "Patch {} [{}]: {}",
            record.patch_id,
            format_patch_status(record.patch.status),
            record.patch.title
        )?;
        writeln!(
            writer,
            "Repository: {}",
            record.patch.service_repo_name.as_str()
        )?;
        if let Some(ref branch) = record.patch.branch_name {
            writeln!(writer, "Branch: {branch}")?;
        }
        writeln!(writer)?;
    }
    writer.flush()?;
    Ok(())
}

fn format_patch_status(status: PatchStatus) -> &'static str {
    match status {
        PatchStatus::Open => "open",
        PatchStatus::Closed => "closed",
        PatchStatus::Merged => "merged",
        PatchStatus::ChangesRequested => "changes requested",
        _ => "unknown",
    }
}

fn render_session_records_jsonl(
    jobs: &[SessionVersionRecord],
    writer: &mut impl Write,
) -> Result<()> {
    for job in jobs {
        serde_json::to_writer(&mut *writer, job)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn render_session_records_pretty(
    jobs: &[SessionVersionRecord],
    writer: &mut impl Write,
) -> Result<()> {
    if jobs.is_empty() {
        writeln!(writer, "No Hydra sessions found.")?;
        writer.flush()?;
        return Ok(());
    }

    let terminal_width = current_terminal_width();

    let (plain_header, colored_header) = header_row();
    writeln!(writer, "{colored_header}")?;
    writeln!(writer, "{}", "-".repeat(plain_header.len()))?;

    let now = Utc::now();
    for job in jobs {
        let status_display = format_status(&job.session.status);
        let runtime = format_runtime(&job.session, now).unwrap_or_else(|| "-".into());
        let notes = session_note(job).unwrap_or_else(|| "-".into());
        let cells = session_row_cells(job.session_id.as_ref(), status_display, &runtime);
        let plain_prefix = session_row_prefix(&cells);
        let colored_prefix = colored_session_row_prefix(&cells, &job.session.status);
        for (index, line) in format_session_lines(&plain_prefix, &notes, terminal_width)
            .into_iter()
            .enumerate()
        {
            if index == 0 {
                let note_body = line.strip_prefix(&plain_prefix).unwrap_or(&line);
                writeln!(writer, "{colored_prefix}{note_body}")?;
            } else {
                writeln!(writer, "{line}")?;
            }
        }
    }
    writer.flush()?;
    Ok(())
}

fn render_session_summary_records_jsonl(
    jobs: &[SessionSummaryRecord],
    writer: &mut impl Write,
) -> Result<()> {
    for job in jobs {
        serde_json::to_writer(&mut *writer, job)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn render_session_summary_records_pretty(
    jobs: &[SessionSummaryRecord],
    writer: &mut impl Write,
) -> Result<()> {
    if jobs.is_empty() {
        writeln!(writer, "No Hydra sessions found.")?;
        writer.flush()?;
        return Ok(());
    }

    let terminal_width = current_terminal_width();

    let (plain_header, colored_header) = header_row();
    writeln!(writer, "{colored_header}")?;
    writeln!(writer, "{}", "-".repeat(plain_header.len()))?;

    let now = Utc::now();
    for job in jobs {
        let status_display = format_status(&job.session.status);
        let runtime = format_summary_runtime(&job.session, now).unwrap_or_else(|| "-".into());
        let notes = session_summary_note(job).unwrap_or_else(|| "-".into());
        let cells = session_row_cells(job.session_id.as_ref(), status_display, &runtime);
        let plain_prefix = session_row_prefix(&cells);
        let colored_prefix = colored_session_row_prefix(&cells, &job.session.status);
        for (index, line) in format_session_lines(&plain_prefix, &notes, terminal_width)
            .into_iter()
            .enumerate()
        {
            if index == 0 {
                let note_body = line.strip_prefix(&plain_prefix).unwrap_or(&line);
                writeln!(writer, "{colored_prefix}{note_body}")?;
            } else {
                writeln!(writer, "{line}")?;
            }
        }
    }
    writer.flush()?;
    Ok(())
}

fn render_agent_records_jsonl(agents: &[AgentRecord], writer: &mut impl Write) -> Result<()> {
    for agent in agents {
        serde_json::to_writer(&mut *writer, agent)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn render_agent_records_pretty(agents: &[AgentRecord], writer: &mut impl Write) -> Result<()> {
    if agents.is_empty() {
        writeln!(writer, "No agents configured.")?;
        writer.flush()?;
        return Ok(());
    }

    for (index, agent) in agents.iter().enumerate() {
        write_agent_details(agent, writer)?;
        if index + 1 < agents.len() {
            writeln!(writer)?;
        }
    }
    writer.flush()?;
    Ok(())
}

fn write_agent_details(agent: &AgentRecord, writer: &mut impl Write) -> Result<()> {
    writeln!(writer, "- {}", agent.name)?;
    if !agent.prompt_path.is_empty() {
        writeln!(writer, "  prompt_path: {}", agent.prompt_path)?;
    }
    if !agent.prompt.is_empty() {
        writeln!(writer, "  prompt: {}", agent.prompt)?;
    }
    writeln!(writer, "  max_tries: {}", agent.max_tries)?;
    writeln!(writer, "  max_simultaneous: {}", agent.max_simultaneous)?;
    writeln!(
        writer,
        "  is_assignment_agent: {}",
        agent.is_assignment_agent
    )?;
    if !agent.secrets.is_empty() {
        writeln!(writer, "  secrets: {}", agent.secrets.join(", "))?;
    }
    Ok(())
}

fn render_repository_records_jsonl(
    repositories: &[RepositoryRecord],
    writer: &mut impl Write,
) -> Result<()> {
    for repository in repositories {
        serde_json::to_writer(&mut *writer, repository)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn render_repository_records_pretty(
    repositories: &[RepositoryRecord],
    writer: &mut impl Write,
) -> Result<()> {
    if repositories.is_empty() {
        writeln!(writer, "No repositories configured.")?;
        writer.flush()?;
        return Ok(());
    }

    for (index, repository) in repositories.iter().enumerate() {
        write_repository_details(repository, writer)?;
        if index + 1 < repositories.len() {
            writeln!(writer)?;
        }
    }
    writer.flush()?;
    Ok(())
}

fn write_repository_details(repository: &RepositoryRecord, writer: &mut impl Write) -> Result<()> {
    let config = &repository.repository;
    writeln!(writer, "- {}", repository.name)?;
    writeln!(writer, "  remote_url: {}", config.remote_url)?;
    writeln!(
        writer,
        "  default_branch: {}",
        config.default_branch.as_deref().unwrap_or("<none>")
    )?;
    writeln!(
        writer,
        "  default_image: {}",
        config.default_image.as_deref().unwrap_or("<none>")
    )?;
    if let Some(ref pw) = config.patch_workflow {
        if !pw.review_requests.is_empty() {
            let reviewers: Vec<&str> = pw
                .review_requests
                .iter()
                .map(|r| r.assignee.as_str())
                .collect();
            writeln!(writer, "  reviewers: {}", reviewers.join(", "))?;
        }
        if let Some(ref mr) = pw.merge_request {
            writeln!(
                writer,
                "  merger: {}",
                mr.assignee.as_deref().unwrap_or("<none>")
            )?;
        }
    }
    Ok(())
}

fn render_document_records_jsonl(
    documents: &[DocumentVersionRecord],
    writer: &mut impl Write,
) -> Result<()> {
    for document in documents {
        serde_json::to_writer(&mut *writer, document)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn render_document_summary_records_jsonl(
    documents: &[DocumentSummaryRecord],
    writer: &mut impl Write,
) -> Result<()> {
    for document in documents {
        serde_json::to_writer(&mut *writer, document)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn render_document_summary_records_pretty(
    documents: &[DocumentSummaryRecord],
    writer: &mut impl Write,
) -> Result<()> {
    if documents.is_empty() {
        writeln!(writer, "No documents found.")?;
        writer.flush()?;
        return Ok(());
    }

    for (index, record) in documents.iter().enumerate() {
        writeln!(writer, "Document {}", record.document_id)?;
        writeln!(writer, "Title: {}", record.document.title)?;
        let path = record.document.path.as_deref().unwrap_or("-");
        writeln!(writer, "Path: {path}")?;
        let created_by = record
            .document
            .created_by
            .as_ref()
            .map(|id| id.as_ref())
            .unwrap_or("-");
        writeln!(writer, "Created by: {created_by}")?;

        if index + 1 < documents.len() {
            writeln!(writer)?;
        }
    }
    writer.flush()?;
    Ok(())
}

fn render_document_records_pretty(
    documents: &[DocumentVersionRecord],
    full_output: bool,
    writer: &mut impl Write,
) -> Result<()> {
    if documents.is_empty() {
        writeln!(writer, "No documents found.")?;
        writer.flush()?;
        return Ok(());
    }

    for (index, record) in documents.iter().enumerate() {
        writeln!(writer, "Document {}", record.document_id)?;
        writeln!(writer, "Title: {}", record.document.title)?;
        let path = record.document.path.as_deref().unwrap_or("-");
        writeln!(writer, "Path: {path}")?;
        let created_by = record
            .document
            .created_by
            .as_ref()
            .map(|id| id.as_ref())
            .unwrap_or("-");
        writeln!(writer, "Created by: {created_by}")?;
        writeln!(writer, "Body:")?;

        let lines: Vec<String> = record
            .document
            .body_markdown
            .lines()
            .map(|line| line.to_string())
            .collect();
        if lines.is_empty() {
            writeln!(writer, "  -")?;
        } else {
            let output_lines = if full_output {
                lines
            } else {
                truncate_lines(lines, MAX_DOCUMENT_BODY_LINES, MAX_DOCUMENT_BODY_WIDTH)
            };
            for line in output_lines {
                if line.is_empty() {
                    writeln!(writer, "  ")?;
                } else {
                    writeln!(writer, "  {line}")?;
                }
            }
        }

        if index + 1 < documents.len() {
            writeln!(writer)?;
        }
    }
    writer.flush()?;
    Ok(())
}

fn render_versioned_messages_jsonl(
    messages: &[VersionedMessage],
    writer: &mut impl Write,
) -> Result<()> {
    for message in messages {
        serde_json::to_writer(&mut *writer, message)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn render_versioned_messages_pretty(
    messages: &[VersionedMessage],
    writer: &mut impl Write,
) -> Result<()> {
    if messages.is_empty() {
        writeln!(writer, "No messages found.")?;
        writer.flush()?;
        return Ok(());
    }

    for (index, msg) in messages.iter().enumerate() {
        writeln!(writer, "Message {} (v{})", msg.message_id, msg.version)?;
        if let Some(ref sender) = msg.message.sender {
            writeln!(writer, "  sender: {sender}")?;
        }
        writeln!(writer, "  recipient: {}", msg.message.recipient)?;
        writeln!(writer, "  timestamp: {}", msg.timestamp)?;
        writeln!(writer, "  body: {}", msg.message.body)?;
        if index + 1 < messages.len() {
            writeln!(writer)?;
        }
    }
    writer.flush()?;
    Ok(())
}

struct SessionRowCells {
    id: String,
    status: String,
    runtime: String,
}

fn session_row_cells(id: &str, status: &str, runtime: &str) -> SessionRowCells {
    SessionRowCells {
        id: format!("{id:<NAME_WIDTH$}"),
        status: format!("{status:<STATUS_WIDTH$}"),
        runtime: format!("{runtime:<RUNTIME_WIDTH$}"),
    }
}

fn session_row_prefix(cells: &SessionRowCells) -> String {
    format!(
        "{:<name_width$} {:<status_width$} {:<runtime_width$} ",
        cells.id,
        cells.status,
        cells.runtime,
        name_width = NAME_WIDTH,
        status_width = STATUS_WIDTH,
        runtime_width = RUNTIME_WIDTH
    )
}

fn colored_session_row_prefix(cells: &SessionRowCells, status: &Status) -> String {
    format!(
        "{} {} {} ",
        cells.id.bright_cyan(),
        color_status(&cells.status, status),
        cells.runtime.bright_magenta(),
    )
}

fn color_status(padded_status: &str, status: &Status) -> String {
    match status {
        Status::Complete => padded_status.green().to_string(),
        Status::Running => padded_status.yellow().to_string(),
        Status::Failed => padded_status.red().to_string(),
        Status::Pending => padded_status.cyan().to_string(),
        Status::Created => padded_status.bold().to_string(),
        _ => padded_status.to_string(),
    }
}

fn format_status(status: &Status) -> &'static str {
    match status {
        Status::Created => "created",
        Status::Pending => "pending",
        Status::Running => "running",
        Status::Complete => "complete",
        Status::Failed => "failed",
        _ => "unknown",
    }
}

fn session_note(job: &SessionVersionRecord) -> Option<String> {
    job.session.error.as_ref().map(format_task_error)
}

fn session_summary_note(job: &SessionSummaryRecord) -> Option<String> {
    job.session.error.as_ref().map(format_task_error)
}

fn format_task_error(error: &TaskError) -> String {
    match error {
        TaskError::JobEngineError { reason } => format!("error: {reason}"),
        other => format!("error: {other:?}"),
    }
}

fn current_terminal_width() -> usize {
    let width = termwidth();
    if width == 0 {
        DEFAULT_TERMINAL_WIDTH
    } else {
        width
    }
}

fn header_row() -> (String, String) {
    let cells = session_row_cells("ID", "STATUS", "RUNTIME");
    let plain = format!(
        "{} {} {} {}",
        cells.id, cells.status, cells.runtime, "NOTES"
    );
    let colored = format!(
        "{} {} {} {}",
        cells.id.bold(),
        cells.status.bold(),
        cells.runtime.bold(),
        "NOTES".bold()
    );
    (plain, colored)
}

fn format_session_lines(prefix: &str, notes: &str, terminal_width: usize) -> Vec<String> {
    let indent = " ".repeat(prefix.len());
    let available_width = terminal_width.saturating_sub(prefix.len()).max(1);
    let notes_width = available_width.min(MAX_NOTES_WIDTH);
    let wrapped_notes = textwrap::wrap(
        notes,
        Options::new(notes_width)
            .break_words(true)
            .wrap_algorithm(WrapAlgorithm::FirstFit),
    )
    .into_iter()
    .map(|line| line.into_owned())
    .collect();
    let wrapped_notes = truncate_lines(wrapped_notes, MAX_NOTE_LINES, notes_width);

    if wrapped_notes.is_empty() {
        vec![format!("{prefix}-")]
    } else {
        wrapped_notes
            .into_iter()
            .enumerate()
            .map(|(index, line)| {
                if index == 0 {
                    format!("{prefix}{line}")
                } else {
                    format!("{indent}{line}")
                }
            })
            .collect()
    }
}

pub(crate) fn format_runtime(task: &Session, now: DateTime<Utc>) -> Option<String> {
    format_runtime_fields(
        task.status,
        task.start_time,
        task.creation_time,
        task.end_time,
        now,
    )
}

fn format_summary_runtime(summary: &SessionSummary, now: DateTime<Utc>) -> Option<String> {
    format_runtime_fields(
        summary.status,
        summary.start_time,
        summary.creation_time,
        summary.end_time,
        now,
    )
}

pub(crate) fn format_runtime_fields(
    status: Status,
    start_time: Option<DateTime<Utc>>,
    creation_time: Option<DateTime<Utc>>,
    end_time: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Option<String> {
    match status {
        Status::Running => {
            // Running: elapsed = now - start_time (or creation_time as fallback)
            let started = start_time.or(creation_time)?;
            let duration = if now < started {
                ChronoDuration::zero()
            } else {
                now - started
            };
            Some(format_duration(duration))
        }
        Status::Pending | Status::Created => {
            // Pending/Created: elapsed = now - creation_time
            let created = creation_time?;
            let duration = if now < created {
                ChronoDuration::zero()
            } else {
                now - created
            };
            Some(format_duration(duration))
        }
        Status::Complete | Status::Failed => {
            // Completed/Failed: total runtime = end_time - start_time (or creation_time)
            let started = start_time.or(creation_time)?;
            let ended = end_time?;
            let duration = if ended < started {
                ChronoDuration::zero()
            } else {
                ended - started
            };
            Some(format_duration(duration))
        }
        _ => None,
    }
}

pub fn render_notifications(
    format: ResolvedOutputFormat,
    response: &ListNotificationsResponse,
    writer: &mut impl Write,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => {
            for notification in &response.notifications {
                serde_json::to_writer(&mut *writer, notification)?;
                writer.write_all(b"\n")?;
            }
            writer.flush()?;
        }
        ResolvedOutputFormat::Pretty => {
            if response.notifications.is_empty() {
                writeln!(writer, "No notifications.")?;
            } else {
                for (index, notification) in response.notifications.iter().enumerate() {
                    write_notification_pretty(notification, writer)?;
                    if index + 1 < response.notifications.len() {
                        writeln!(writer)?;
                    }
                }
            }
            writer.flush()?;
        }
    }
    Ok(())
}

fn write_notification_pretty(record: &NotificationResponse, writer: &mut impl Write) -> Result<()> {
    let read_status = if record.notification.is_read {
        "read"
    } else {
        "unread"
    };
    writeln!(
        writer,
        "Notification {} [{}]",
        record.notification_id, read_status
    )?;
    writeln!(writer, "  summary: {}", record.notification.summary)?;
    writeln!(
        writer,
        "  object: {} {}",
        record.notification.object_kind, record.notification.object_id
    )?;
    writeln!(writer, "  event: {}", record.notification.event_type)?;
    if let Some(ref source) = record.notification.source_actor {
        writeln!(writer, "  source: {source}")?;
    }
    writeln!(writer, "  time: {}", record.notification.created_at)?;
    Ok(())
}

pub fn render_unread_count(
    format: ResolvedOutputFormat,
    response: &UnreadCountResponse,
    writer: &mut impl Write,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => {
            serde_json::to_writer(&mut *writer, response)?;
            writer.write_all(b"\n")?;
            writer.flush()?;
        }
        ResolvedOutputFormat::Pretty => {
            writeln!(writer, "{} unread notifications.", response.count)?;
            writer.flush()?;
        }
    }
    Ok(())
}

pub fn render_mark_read(
    format: ResolvedOutputFormat,
    notification_id: &NotificationId,
    response: &MarkReadResponse,
    writer: &mut impl Write,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => {
            serde_json::to_writer(&mut *writer, response)?;
            writer.write_all(b"\n")?;
            writer.flush()?;
        }
        ResolvedOutputFormat::Pretty => {
            writeln!(writer, "Notification {notification_id} marked as read.")?;
            writer.flush()?;
        }
    }
    Ok(())
}

pub fn render_mark_all_read(
    format: ResolvedOutputFormat,
    response: &MarkReadResponse,
    writer: &mut impl Write,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => {
            serde_json::to_writer(&mut *writer, response)?;
            writer.write_all(b"\n")?;
            writer.flush()?;
        }
        ResolvedOutputFormat::Pretty => {
            writeln!(writer, "{} notifications marked as read.", response.marked)?;
            writer.flush()?;
        }
    }
    Ok(())
}

pub fn render_relations(
    format: ResolvedOutputFormat,
    response: &hydra_common::api::v1::relations::ListRelationsResponse,
    writer: &mut impl Write,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => {
            for relation in &response.relations {
                serde_json::to_writer(&mut *writer, relation)?;
                writer.write_all(b"\n")?;
            }
            writer.flush()?;
        }
        ResolvedOutputFormat::Pretty => {
            if response.relations.is_empty() {
                writeln!(writer, "No relations found.")?;
            } else {
                let source_w = response
                    .relations
                    .iter()
                    .map(|r| r.source_id.to_string().len())
                    .max()
                    .unwrap_or(6)
                    .max(6);
                let rel_w = response
                    .relations
                    .iter()
                    .map(|r| r.rel_type.len())
                    .max()
                    .unwrap_or(8)
                    .max(8);

                writeln!(
                    writer,
                    "{:<source_w$}  {:<rel_w$}  TARGET",
                    "SOURCE", "REL TYPE"
                )?;
                writeln!(
                    writer,
                    "{:<source_w$}  {:<rel_w$}  {}",
                    "-".repeat(source_w),
                    "-".repeat(rel_w),
                    "-".repeat(6)
                )?;
                for relation in &response.relations {
                    writeln!(
                        writer,
                        "{:<source_w$}  {:<rel_w$}  {}",
                        relation.source_id, relation.rel_type, relation.target_id
                    )?;
                }
            }
            writer.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::HydraClient;
    use chrono::Utc;
    use httpmock::prelude::*;
    use hydra_common::{
        documents::{Document, DocumentVersionRecord},
        whoami::WhoAmIResponse,
        DocumentId, SessionId,
    };
    use std::str::FromStr;

    const TEST_HYDRA_TOKEN: &str = "test-hydra-token";

    #[tokio::test]
    async fn resolve_output_format_auto_prefers_pretty_for_users() {
        let server = MockServer::start();
        let client = HydraClient::new(server.base_url(), TEST_HYDRA_TOKEN).expect("client");
        let whoami = WhoAmIResponse::new(ActorIdentity::User {
            username: "user".into(),
        });

        let mock = server.mock(|when, then| {
            when.method(GET).path("/v1/whoami");
            then.status(200).json_body_obj(&whoami);
        });

        let resolved = resolve_output_format(&client, OutputFormat::Auto)
            .await
            .expect("resolve output format");

        mock.assert();
        assert_eq!(resolved, ResolvedOutputFormat::Pretty);
    }

    #[tokio::test]
    async fn resolve_output_format_auto_prefers_jsonl_for_tasks() {
        let server = MockServer::start();
        let client = HydraClient::new(server.base_url(), TEST_HYDRA_TOKEN).expect("client");
        let whoami = WhoAmIResponse::new(ActorIdentity::Session {
            session_id: SessionId::from_str("s-task").expect("task id"),
            creator: "test-creator".into(),
        });

        let mock = server.mock(|when, then| {
            when.method(GET).path("/v1/whoami");
            then.status(200).json_body_obj(&whoami);
        });

        let resolved = resolve_output_format(&client, OutputFormat::Auto)
            .await
            .expect("resolve output format");

        mock.assert();
        assert_eq!(resolved, ResolvedOutputFormat::Jsonl);
    }

    #[test]
    fn wraps_notes_to_terminal_width_and_indents_followup_lines() {
        let cells = session_row_cells("job-123", "running", "12s");
        let prefix = session_row_prefix(&cells);
        let terminal_width = prefix.len() + 80;
        let notes =
            "This is a long note that should wrap to the next line when it exceeds the terminal width.";

        let lines = format_session_lines(&prefix, notes, terminal_width);
        let wrapped_notes = textwrap::wrap(
            notes,
            Options::new(MAX_NOTES_WIDTH)
                .break_words(true)
                .wrap_algorithm(WrapAlgorithm::FirstFit),
        );

        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with(&prefix));
        assert!(lines[1].starts_with(&" ".repeat(prefix.len())));
        assert_eq!(lines[0], format!("{prefix}{}", wrapped_notes[0]));
        assert_eq!(
            lines[1],
            format!("{}{}", " ".repeat(prefix.len()), wrapped_notes[1])
        );
    }

    #[test]
    fn caps_notes_width_when_terminal_is_wide() {
        let cells = session_row_cells("job-123", "running", "12s");
        let prefix = session_row_prefix(&cells);
        let terminal_width = 400;
        let notes = "a".repeat(170);

        let lines = format_session_lines(&prefix, &notes, terminal_width);

        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].len() - prefix.len(), MAX_NOTES_WIDTH);
        assert!(lines
            .iter()
            .all(|line| line.len() - prefix.len() <= MAX_NOTES_WIDTH));
    }

    #[test]
    fn notes_are_truncated_after_five_lines() {
        let cells = session_row_cells("job-123", "running", "12s");
        let prefix = session_row_prefix(&cells);
        let terminal_width = prefix.len() + 20;
        let notes = "word ".repeat(120);

        let lines = format_session_lines(&prefix, &notes, terminal_width);

        assert_eq!(lines.len(), MAX_NOTE_LINES);
        assert!(lines.last().unwrap().contains("..."));
    }

    #[test]
    fn format_status_returns_plain_labels() {
        assert_eq!(format_status(&Status::Created), "created");
        assert_eq!(format_status(&Status::Pending), "pending");
        assert_eq!(format_status(&Status::Running), "running");
        assert_eq!(format_status(&Status::Complete), "complete");
        assert_eq!(format_status(&Status::Failed), "failed");
    }

    #[test]
    fn test_render_relations_jsonl() {
        use hydra_common::api::v1::relations::{ListRelationsResponse, RelationResponse};

        let response = ListRelationsResponse {
            relations: vec![
                RelationResponse {
                    source_id: "i-aaaaaa".parse().unwrap(),
                    target_id: "i-bbbbbb".parse().unwrap(),
                    rel_type: "child-of".to_string(),
                },
                RelationResponse {
                    source_id: "i-cccccc".parse().unwrap(),
                    target_id: "p-dddddd".parse().unwrap(),
                    rel_type: "has-patch".to_string(),
                },
            ],
        };
        let mut buf = Vec::new();
        render_relations(ResolvedOutputFormat::Jsonl, &response, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("\"source_id\":\"i-aaaaaa\""));
        assert!(output.contains("\"target_id\":\"i-bbbbbb\""));
        assert!(output.contains("\"rel_type\":\"child-of\""));
        assert!(output.contains("\"source_id\":\"i-cccccc\""));
        assert!(output.contains("\"rel_type\":\"has-patch\""));
        // Each relation should be on its own line
        assert_eq!(output.lines().count(), 2);
    }

    #[test]
    fn test_render_relations_pretty() {
        use hydra_common::api::v1::relations::{ListRelationsResponse, RelationResponse};

        let response = ListRelationsResponse {
            relations: vec![
                RelationResponse {
                    source_id: "i-aaaaaa".parse().unwrap(),
                    target_id: "i-bbbbbb".parse().unwrap(),
                    rel_type: "child-of".to_string(),
                },
                RelationResponse {
                    source_id: "i-cccccc".parse().unwrap(),
                    target_id: "p-dddddd".parse().unwrap(),
                    rel_type: "has-patch".to_string(),
                },
            ],
        };
        let mut buf = Vec::new();
        render_relations(ResolvedOutputFormat::Pretty, &response, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("SOURCE"));
        assert!(output.contains("REL TYPE"));
        assert!(output.contains("TARGET"));
        assert!(output.contains("------"));
        assert!(output.contains("i-aaaaaa"));
        assert!(output.contains("child-of"));
        assert!(output.contains("i-bbbbbb"));
        assert!(output.contains("i-cccccc"));
        assert!(output.contains("has-patch"));
        assert!(output.contains("p-dddddd"));
    }

    #[test]
    fn test_render_relations_empty_pretty() {
        use hydra_common::api::v1::relations::ListRelationsResponse;

        let response = ListRelationsResponse { relations: vec![] };
        let mut buf = Vec::new();
        render_relations(ResolvedOutputFormat::Pretty, &response, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("No relations found."));
    }

    #[test]
    fn test_render_relations_empty_jsonl() {
        use hydra_common::api::v1::relations::ListRelationsResponse;

        let response = ListRelationsResponse { relations: vec![] };
        let mut buf = Vec::new();
        render_relations(ResolvedOutputFormat::Jsonl, &response, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.is_empty());
    }

    #[test]
    fn render_document_records_truncates_body_by_default() {
        let mut body_lines = Vec::new();
        for index in 0..25 {
            body_lines.push(format!("line {index:02} {}", "x".repeat(10)));
        }
        let document = Document::new(
            "Doc".to_string(),
            body_lines.join("\n"),
            Some("docs/runbook.md".to_string()),
            Some(SessionId::new()),
            false,
        )
        .unwrap();
        let record = DocumentVersionRecord::new(
            DocumentId::new(),
            0,
            Utc::now(),
            document,
            None,
            Utc::now(),
            Vec::new(),
        );
        let mut output = Vec::new();
        render_document_records(ResolvedOutputFormat::Pretty, &[record], false, &mut output)
            .unwrap();
        let rendered = String::from_utf8(output).unwrap();
        assert!(rendered.contains("Document"));
        assert!(rendered.contains("line 19"));
        assert!(rendered.contains("..."));
        assert!(!rendered.contains("line 24"));
    }

    #[test]
    fn render_document_records_shows_full_body_when_requested() {
        let mut body_lines = Vec::new();
        for index in 0..25 {
            body_lines.push(format!("line {index:02} {}", "x".repeat(10)));
        }
        let document = Document::new(
            "Doc".to_string(),
            body_lines.join("\n"),
            Some("docs/runbook.md".to_string()),
            Some(SessionId::new()),
            false,
        )
        .unwrap();
        let record = DocumentVersionRecord::new(
            DocumentId::new(),
            0,
            Utc::now(),
            document,
            None,
            Utc::now(),
            Vec::new(),
        );
        let mut output = Vec::new();
        render_document_records(ResolvedOutputFormat::Pretty, &[record], true, &mut output)
            .unwrap();
        let rendered = String::from_utf8(output).unwrap();
        assert!(rendered.contains("Document"));
        assert!(rendered.contains("line 00"));
        assert!(rendered.contains("line 24"));
        assert!(!rendered.contains("..."));
    }
}
