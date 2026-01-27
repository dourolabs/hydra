use std::io::Write;

use anyhow::Result;
use clap::ValueEnum;
use metis_common::{
    issues::{Issue, IssueRecord},
    jobs::JobRecord,
    patches::{PatchRecord, PatchStatus},
    whoami::ActorIdentity,
};

use crate::client::MetisClientInterface;

const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const RESET: &str = "\x1b[0m";

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
    for job in jobs {
        writeln!(writer, "Job {}", job.id)?;
        if let Some(notes) = job.notes.as_deref() {
            writeln!(writer, "Notes: {notes}")?;
        }
        writeln!(writer)?;
    }
    writer.flush()?;
    Ok(())
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
}
