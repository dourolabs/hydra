use crate::{client::MetisClientInterface, util::truncate_lines};
use anyhow::Result;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use metis_common::{
    jobs::{JobRecord, SearchJobsQuery},
    task_status::{Status, TaskStatusLog},
    IssueId, TaskId,
};
use owo_colors::OwoColorize;
use serde::{Deserialize, Serialize};
use textwrap::{termwidth, Options, WrapAlgorithm};

const NAME_WIDTH: usize = 48;
const STATUS_WIDTH: usize = 26;
const RUNTIME_WIDTH: usize = 12;
const MAX_NOTES_WIDTH: usize = 80;
const MAX_NOTE_LINES: usize = 5;
const DEFAULT_TERMINAL_WIDTH: usize = 80;
pub const DEFAULT_JOB_LIMIT: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct JobSummary {
    id: TaskId,
    status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    created_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    finished_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct JobsListOutput {
    page: usize,
    page_size: usize,
    total: usize,
    jobs: Vec<JobSummary>,
}

pub async fn run(
    client: &dyn MetisClientInterface,
    limit: usize,
    spawned_from: Option<IssueId>,
    json: bool,
) -> Result<()> {
    let response = client
        .list_jobs(&SearchJobsQuery {
            q: None,
            spawned_from,
        })
        .await?;
    let limit = limit.max(1);
    let total_jobs = response.jobs.len();

    if json {
        let output = jobs_json_output(response.jobs, limit);
        println!("{}", serde_json::to_string(&output)?);
        return Ok(());
    }

    let terminal_width = current_terminal_width();
    let now = Utc::now();

    if total_jobs == 0 {
        println!("No Metis jobs found.");
        return Ok(());
    }

    let (jobs, truncated) = truncate_jobs(response.jobs, limit);
    let (plain_header, colored_header) = header_row();
    println!("{colored_header}");
    println!("{}", "-".repeat(plain_header.len()));

    for job in jobs {
        let status_display = format_status(&job.status_log.current_status());
        let runtime = format_runtime(&job.status_log, now).unwrap_or_else(|| "-".into());
        let notes = job_note(&job).unwrap_or_else(|| "-".into());
        let cells = job_row_cells(job.id.as_ref(), status_display, &runtime);
        let plain_prefix = job_row_prefix(&cells);
        let colored_prefix = colored_job_row_prefix(&cells, &job.status_log.current_status());
        for (index, line) in format_job_lines(&plain_prefix, &notes, terminal_width)
            .into_iter()
            .enumerate()
        {
            if index == 0 {
                let note_body = line.strip_prefix(&plain_prefix).unwrap_or(&line);
                println!("{colored_prefix}{note_body}");
            } else {
                println!("{line}");
            }
        }
    }

    if truncated {
        println!("Showing {limit} of {total_jobs} jobs. Use --limit to display more.");
    }

    Ok(())
}

pub(crate) fn truncate_jobs(jobs: Vec<JobRecord>, limit: usize) -> (Vec<JobRecord>, bool) {
    if jobs.len() <= limit {
        return (jobs, false);
    }

    (jobs.into_iter().take(limit).collect(), true)
}

fn jobs_json_output(jobs: Vec<JobRecord>, limit: usize) -> JobsListOutput {
    let page_size = limit.max(1);
    let total = jobs.len();
    let (jobs, _) = truncate_jobs(jobs, page_size);
    let jobs = jobs.into_iter().map(JobSummary::from).collect();

    JobsListOutput {
        page: 1,
        page_size,
        total,
        jobs,
    }
}

impl From<JobRecord> for JobSummary {
    fn from(job: JobRecord) -> Self {
        Self {
            id: job.id,
            status: job.status_log.current_status(),
            created_at: job.status_log.creation_time(),
            started_at: job.status_log.start_time(),
            finished_at: job.status_log.end_time(),
            notes: job.notes,
        }
    }
}

pub(crate) fn format_job_lines(prefix: &str, notes: &str, terminal_width: usize) -> Vec<String> {
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

pub(crate) fn current_terminal_width() -> usize {
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

fn colored_job_row_prefix(cells: &JobRowCells, status: &Status) -> String {
    format!(
        "{} {} {} ",
        cells.id.bright_cyan(),
        color_status(&cells.status, status),
        cells.runtime.bright_magenta(),
    )
}

pub(crate) fn color_status(padded_status: &str, status: &Status) -> String {
    match status {
        Status::Complete => padded_status.green().to_string(),
        Status::Running => padded_status.yellow().to_string(),
        Status::Failed => padded_status.red().to_string(),
        Status::Pending => padded_status.bold().to_string(),
    }
}

pub(crate) fn format_status(status: &Status) -> &'static str {
    match status {
        Status::Pending => "pending",
        Status::Running => "running",
        Status::Complete => "complete",
        Status::Failed => "failed",
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

pub(crate) fn format_duration(duration: ChronoDuration) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        client::MetisClient,
        test_utils::ids::{issue_id, task_id},
    };
    use chrono::TimeZone;
    use httpmock::prelude::*;
    use metis_common::jobs::{BundleSpec, ListJobsResponse, Task};
    use metis_common::task_status::Event;
    use std::collections::HashMap;

    fn only_spawned_from_query(request: &HttpMockRequest) -> bool {
        match &request.query_params {
            Some(params) => params.len() == 1 && params[0].0 == "spawned_from",
            None => false,
        }
    }

    fn sample_job(id: &str) -> JobRecord {
        JobRecord {
            id: task_id(id),
            task: Task {
                prompt: "0".to_string(),
                context: BundleSpec::None,
                spawned_from: None,
                image: None,
                env_vars: HashMap::new(),
            },
            notes: None,
            status_log: TaskStatusLog::new(Status::Pending, Utc::now()),
        }
    }

    #[test]
    fn truncate_jobs_keeps_all_when_below_limit() {
        let jobs = vec![
            sample_job("t-job-1"),
            sample_job("t-job-2"),
            sample_job("t-job-3"),
        ];

        let (kept, truncated) = truncate_jobs(jobs, 5);

        assert!(!truncated);
        assert_eq!(kept.len(), 3);
        assert_eq!(kept[0].id, task_id("t-job-1"));
        assert_eq!(kept[2].id, task_id("t-job-3"));
    }

    #[test]
    fn truncate_jobs_limits_to_requested_count() {
        let jobs: Vec<JobRecord> = (0..12)
            .map(|idx| sample_job(&format!("t-job-{idx}")))
            .collect();

        let (kept, truncated) = truncate_jobs(jobs, 10);

        assert!(truncated);
        assert_eq!(kept.len(), 10);
        assert_eq!(kept.first().unwrap().id, task_id("t-job-0"));
        assert_eq!(kept.last().unwrap().id, task_id("t-job-9"));
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
        assert_eq!(format_status(&Status::Pending), "pending");
        assert_eq!(format_status(&Status::Running), "running");
        assert_eq!(format_status(&Status::Complete), "complete");
        assert_eq!(format_status(&Status::Failed), "failed");
    }

    #[tokio::test]
    async fn run_passes_spawned_from_query() {
        let spawned_from = issue_id("from-filter");
        let server = MockServer::start();
        let client = MetisClient::new(server.base_url()).expect("should construct client");

        let list_response = ListJobsResponse {
            jobs: vec![sample_job("t-job-1")],
        };

        let mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/jobs/")
                .query_param("spawned_from", spawned_from.as_ref())
                .matches(only_spawned_from_query);
            then.status(200).json_body_obj(&list_response);
        });

        run(&client, 5, Some(spawned_from.clone()), false)
            .await
            .expect("list jobs should succeed");

        mock.assert();
    }

    #[test]
    fn jobs_json_output_includes_timestamps() {
        let created_at = Utc
            .with_ymd_and_hms(2024, 1, 2, 3, 4, 5)
            .single()
            .expect("valid timestamp");
        let started_at = created_at + ChronoDuration::seconds(5);
        let finished_at = started_at + ChronoDuration::seconds(10);

        let mut job = sample_job("t-job-1");
        job.notes = Some("note".to_string());
        job.status_log = TaskStatusLog::new(Status::Pending, created_at);
        job.status_log
            .events
            .push(Event::Started { at: started_at });
        job.status_log.events.push(Event::Completed {
            at: finished_at,
            last_message: None,
        });

        let output = jobs_json_output(vec![job], 5);
        let serialized = serde_json::to_string(&output).expect("should serialize");
        let parsed: JobsListOutput =
            serde_json::from_str(&serialized).expect("should parse serialized output");

        assert_eq!(parsed.total, 1);
        assert_eq!(parsed.page, 1);
        assert_eq!(parsed.page_size, 5);
        assert_eq!(parsed.jobs.len(), 1);

        let summary = &parsed.jobs[0];
        assert_eq!(summary.id, task_id("t-job-1"));
        assert_eq!(summary.status, Status::Complete);
        assert_eq!(summary.created_at, Some(created_at));
        assert_eq!(summary.started_at, Some(started_at));
        assert_eq!(summary.finished_at, Some(finished_at));
        assert_eq!(summary.notes.as_deref(), Some("note"));
    }

    #[test]
    fn jobs_json_output_handles_empty_results() {
        let output = jobs_json_output(Vec::new(), 3);
        let serialized = serde_json::to_string(&output).expect("should serialize");
        let parsed: JobsListOutput =
            serde_json::from_str(&serialized).expect("should parse serialized output");

        assert_eq!(parsed.total, 0);
        assert_eq!(parsed.page, 1);
        assert_eq!(parsed.page_size, 3);
        assert!(parsed.jobs.is_empty());
    }
}
