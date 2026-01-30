use std::io::Write;

use anyhow::Result;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use clap::ValueEnum;
use metis_common::{
    agents::AgentRecord,
    issues::{Issue, IssueRecord},
    jobs::JobRecord,
    patches::{PatchRecord, PatchStatus},
    repositories::RepositoryRecord,
    task_status::{Status, TaskStatusLog},
    whoami::ActorIdentity,
};
use owo_colors::OwoColorize;
use textwrap::{termwidth, Options, WrapAlgorithm};

use crate::client::MetisClientInterface;
use crate::util::truncate_lines;

const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const RESET: &str = "\x1b[0m";
const NAME_WIDTH: usize = 48;
const STATUS_WIDTH: usize = 26;
const RUNTIME_WIDTH: usize = 12;
const MAX_NOTES_WIDTH: usize = 80;
const MAX_NOTE_LINES: usize = 5;
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
    client: &dyn MetisClientInterface,
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
    issues: &[IssueRecord],
    writer: &mut impl Write,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => render_issue_records_jsonl(issues, writer),
        ResolvedOutputFormat::Pretty => render_issue_records_pretty(issues, writer),
    }
}

pub fn render_patch_records(
    format: ResolvedOutputFormat,
    patches: &[PatchRecord],
    writer: &mut impl Write,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => render_patch_records_jsonl(patches, writer),
        ResolvedOutputFormat::Pretty => render_patch_records_pretty(patches, writer),
    }
}

pub fn render_job_records(
    format: ResolvedOutputFormat,
    jobs: &[JobRecord],
    writer: &mut impl Write,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => render_job_records_jsonl(jobs, writer),
        ResolvedOutputFormat::Pretty => render_job_records_pretty(jobs, writer),
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

async fn resolve_auto_output_format(
    client: &dyn MetisClientInterface,
) -> Result<ResolvedOutputFormat> {
    let whoami = client.whoami().await?;
    Ok(match whoami.actor {
        ActorIdentity::User { .. } => ResolvedOutputFormat::Pretty,
        ActorIdentity::Task { .. } => ResolvedOutputFormat::Jsonl,
        _ => ResolvedOutputFormat::Jsonl,
    })
}

fn render_issue_records_jsonl(issues: &[IssueRecord], writer: &mut impl Write) -> Result<()> {
    for issue in issues {
        serde_json::to_writer(&mut *writer, issue)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn render_issue_records_pretty(issues: &[IssueRecord], writer: &mut impl Write) -> Result<()> {
    for (index, issue_record) in issues.iter().enumerate() {
        let Issue {
            issue_type,
            description,
            creator,
            progress,
            status,
            assignee,
            dependencies,
            ..
        } = &issue_record.issue;

        writeln!(writer, "Issue {} ({issue_type}, {status})", issue_record.id)?;
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

fn render_patch_records_jsonl(patches: &[PatchRecord], writer: &mut impl Write) -> Result<()> {
    for patch in patches {
        serde_json::to_writer(&mut *writer, patch)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn render_patch_records_pretty(patches: &[PatchRecord], writer: &mut impl Write) -> Result<()> {
    for patch in patches {
        write_patch_record_pretty(patch, writer)?;
    }
    writer.flush()?;
    Ok(())
}

fn write_patch_record_pretty(record: &PatchRecord, writer: &mut impl Write) -> Result<()> {
    let title = extract_patch_title(record);
    let status = extract_patch_status(record);
    let description = extract_patch_description(record);
    writeln!(
        writer,
        "Patch {} [{}]: {}",
        record.id,
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

fn extract_patch_title(record: &PatchRecord) -> &str {
    record.patch.title.as_str()
}

fn extract_patch_status(record: &PatchRecord) -> PatchStatus {
    record.patch.status
}

fn extract_patch_description(record: &PatchRecord) -> &str {
    record.patch.description.as_str()
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

fn render_job_records_jsonl(jobs: &[JobRecord], writer: &mut impl Write) -> Result<()> {
    for job in jobs {
        serde_json::to_writer(&mut *writer, job)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn render_job_records_pretty(jobs: &[JobRecord], writer: &mut impl Write) -> Result<()> {
    if jobs.is_empty() {
        writeln!(writer, "No Metis jobs found.")?;
        writer.flush()?;
        return Ok(());
    }

    let terminal_width = current_terminal_width();
    let now = Utc::now();

    let (plain_header, colored_header) = header_row();
    writeln!(writer, "{colored_header}")?;
    writeln!(writer, "{}", "-".repeat(plain_header.len()))?;

    for job in jobs {
        let status_display = format_status(&job.status_log.current_status());
        let runtime = format_runtime(&job.status_log, now).unwrap_or_else(|| "-".into());
        let notes = job_note(job).unwrap_or_else(|| "-".into());
        let cells = job_row_cells(job.id.as_ref(), status_display, &runtime);
        let plain_prefix = job_row_prefix(&cells);
        let colored_prefix = colored_job_row_prefix(&cells, &job.status_log.current_status());
        for (index, line) in format_job_lines(&plain_prefix, &notes, terminal_width)
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
    writeln!(writer, "  prompt: {}", agent.prompt)?;
    writeln!(writer, "  max_tries: {}", agent.max_tries)?;
    writeln!(writer, "  max_simultaneous: {}", agent.max_simultaneous)?;
    writeln!(writer, "  match_unassigned: {}", agent.match_unassigned)?;
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
    Ok(())
}

struct JobRowCells {
    id: String,
    status: String,
    runtime: String,
}

fn job_row_cells(id: &str, status: &str, runtime: &str) -> JobRowCells {
    JobRowCells {
        id: format!("{id:<NAME_WIDTH$}"),
        status: format!("{status:<STATUS_WIDTH$}"),
        runtime: format!("{runtime:<RUNTIME_WIDTH$}"),
    }
}

fn job_row_prefix(cells: &JobRowCells) -> String {
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

fn colored_job_row_prefix(cells: &JobRowCells, status: &Status) -> String {
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

pub(crate) fn format_runtime(status_log: &TaskStatusLog, now: DateTime<Utc>) -> Option<String> {
    let start = status_log.start_time().or(status_log.creation_time())?;
    let end = status_log.end_time().unwrap_or(now);
    let duration = if end < start {
        ChronoDuration::zero()
    } else {
        end - start
    };

    Some(format_duration(duration))
}

fn format_duration(duration: ChronoDuration) -> String {
    let total_seconds = duration.num_seconds();
    if total_seconds <= 0 {
        return "0s".to_string();
    }

    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    if hours > 0 {
        format!("{hours}h {minutes:02}m {seconds:02}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

fn job_note(job: &JobRecord) -> Option<String> {
    job.notes.clone()
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
    let cells = job_row_cells("ID", "STATUS", "RUNTIME");
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

fn format_job_lines(prefix: &str, notes: &str, terminal_width: usize) -> Vec<String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MetisClient;
    use httpmock::prelude::*;
    use metis_common::{whoami::WhoAmIResponse, TaskId};
    use std::str::FromStr;

    const TEST_METIS_TOKEN: &str = "test-metis-token";

    #[tokio::test]
    async fn resolve_output_format_auto_prefers_pretty_for_users() {
        let server = MockServer::start();
        let client = MetisClient::new(server.base_url(), TEST_METIS_TOKEN).expect("client");
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
        let client = MetisClient::new(server.base_url(), TEST_METIS_TOKEN).expect("client");
        let whoami = WhoAmIResponse::new(ActorIdentity::Task {
            task_id: TaskId::from_str("t-task").expect("task id"),
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
        let cells = job_row_cells("job-123", "running", "12s");
        let prefix = job_row_prefix(&cells);
        let terminal_width = prefix.len() + 80;
        let notes =
            "This is a long note that should wrap to the next line when it exceeds the terminal width.";

        let lines = format_job_lines(&prefix, notes, terminal_width);
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
        let cells = job_row_cells("job-123", "running", "12s");
        let prefix = job_row_prefix(&cells);
        let terminal_width = 400;
        let notes = "a".repeat(170);

        let lines = format_job_lines(&prefix, &notes, terminal_width);

        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].len() - prefix.len(), MAX_NOTES_WIDTH);
        assert!(lines
            .iter()
            .all(|line| line.len() - prefix.len() <= MAX_NOTES_WIDTH));
    }

    #[test]
    fn notes_are_truncated_after_five_lines() {
        let cells = job_row_cells("job-123", "running", "12s");
        let prefix = job_row_prefix(&cells);
        let terminal_width = prefix.len() + 20;
        let notes = "word ".repeat(120);

        let lines = format_job_lines(&prefix, &notes, terminal_width);

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
}
