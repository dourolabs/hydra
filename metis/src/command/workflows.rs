use crate::{
    client::MetisClientInterface,
    command::jobs::{
        color_status, current_terminal_width, format_runtime, format_status_with_finished,
    },
};
use anyhow::Result;
use chrono::{DateTime, Utc};
use metis_common::{
    task_status::{Status, TaskStatusLog},
    workflows::WorkflowSummary,
};
use owo_colors::OwoColorize;
use textwrap::{Options, WrapAlgorithm};

const NAME_WIDTH: usize = 36;
const STATUS_WIDTH: usize = 26;
const START_WIDTH: usize = 20;
const RUNTIME_WIDTH: usize = 12;
const RUNNING_WIDTH: usize = 18;
const TEXT_COLUMN_WIDTH: usize = 80;
const MAX_PROMPT_LINES: usize = 5;

pub async fn run(client: &dyn MetisClientInterface) -> Result<()> {
    let response = client.list_workflows().await?;
    let terminal_width = current_terminal_width();
    let now = Utc::now();

    if response.workflows.is_empty() {
        println!("No Metis workflows found.");
        return Ok(());
    }

    let lines = render_workflows(&response.workflows, terminal_width, now);

    for line in lines {
        println!("{line}");
    }

    Ok(())
}

fn render_workflows(
    workflows: &[WorkflowSummary],
    terminal_width: usize,
    now: chrono::DateTime<Utc>,
) -> Vec<String> {
    let mut rendered = Vec::new();
    let (plain_header, colored_header) = header_rows(terminal_width);
    rendered.extend(colored_header);
    if let Some(first_header) = plain_header.first() {
        rendered.push("-".repeat(first_header.len()));
    }

    for workflow in workflows {
        let status_display = format_status_with_finished(&workflow.status_log, now);
        let runtime = format_runtime(&workflow.status_log, now).unwrap_or_else(|| "-".into());
        let start_time = format_start_time(&workflow.status_log);
        let running = running_tasks_display(&workflow.running_tasks);
        let prompt = workflow_prompt(workflow);

        let cells = workflow_row_cells(
            &workflow.id,
            &status_display,
            &start_time,
            &runtime,
            &running,
        );
        let plain_prefix = workflow_row_prefix(&cells);
        let colored_prefix = colored_workflow_row_prefix(
            &cells,
            &workflow.status,
            &running,
            &workflow.running_tasks,
        );

        for (index, line) in format_workflow_lines(&plain_prefix, &prompt, terminal_width)
            .into_iter()
            .enumerate()
        {
            if index == 0 {
                let line_body = line.strip_prefix(&plain_prefix).unwrap_or(&line);
                rendered.push(format!("{colored_prefix}{line_body}"));
            } else {
                rendered.push(line);
            }
        }
    }

    rendered
}

fn header_rows(terminal_width: usize) -> (Vec<String>, Vec<String>) {
    let cells = workflow_row_cells("ID", "STATUS", "STARTED", "RUNTIME", "RUNNING");
    let plain_prefix = workflow_row_prefix(&cells);
    let colored_prefix = format!(
        "{} {} {} {} {} ",
        cells.id.bold(),
        cells.status.bold(),
        cells.start_time.bold(),
        cells.runtime.bold(),
        cells.running.bold(),
    );

    let plain = format_workflow_lines(&plain_prefix, "PROMPT", terminal_width);
    let colored = plain
        .iter()
        .enumerate()
        .map(|(index, line)| {
            if index == 0 {
                let body = line.strip_prefix(&plain_prefix).unwrap_or(line);
                format!(
                    "{}{}",
                    colored_prefix,
                    body.replace("PROMPT", &"PROMPT".bold().to_string())
                )
            } else {
                line.replace("PROMPT", &"PROMPT".bold().to_string())
            }
        })
        .collect();

    (plain, colored)
}

fn workflow_prompt(workflow: &WorkflowSummary) -> String {
    workflow
        .prompt
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("-")
        .to_string()
}

fn running_tasks_display(running_tasks: &[String]) -> String {
    if running_tasks.is_empty() {
        "-".to_string()
    } else {
        let joined = running_tasks.join(", ");
        clamp_text(&joined, RUNNING_WIDTH)
    }
}

fn format_workflow_lines(prefix: &str, prompt: &str, terminal_width: usize) -> Vec<String> {
    let indent = " ".repeat(prefix.len());
    let available_width = terminal_width.saturating_sub(prefix.len()).max(1);

    let prompt_width = available_width.min(TEXT_COLUMN_WIDTH).max(1);
    let prompt_lines = wrap_column(prompt, prompt_width);
    let prompt_lines = truncate_lines(prompt_lines, MAX_PROMPT_LINES, prompt_width);

    prompt_lines
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

fn wrap_column(value: &str, width: usize) -> Vec<String> {
    let display = if value.trim().is_empty() {
        "-".to_string()
    } else {
        value.to_string()
    };

    let wrapped = textwrap::wrap(
        &display,
        Options::new(width.max(1))
            .break_words(true)
            .wrap_algorithm(WrapAlgorithm::FirstFit),
    );

    if wrapped.is_empty() {
        vec!["-".into()]
    } else {
        wrapped.into_iter().map(|line| line.into_owned()).collect()
    }
}

fn truncate_lines(lines: Vec<String>, max_lines: usize, max_width: usize) -> Vec<String> {
    if max_lines == 0 || lines.len() <= max_lines {
        return lines;
    }

    let mut truncated: Vec<String> = lines.into_iter().take(max_lines).collect();
    if let Some(last) = truncated.last_mut() {
        let ellipsis = "...";
        if max_width <= ellipsis.len() {
            *last = ellipsis.chars().take(max_width).collect();
        } else {
            let keep = max_width - ellipsis.len();
            let mut shortened: String = last.chars().take(keep).collect();
            shortened.push_str(ellipsis);
            *last = shortened;
        }
    }

    truncated
}

fn clamp_text(value: &str, max: usize) -> String {
    if value.len() <= max {
        value.to_string()
    } else if max <= 3 {
        ".".repeat(max)
    } else {
        let trimmed: String = value.chars().take(max.saturating_sub(3)).collect();
        format!("{trimmed}...")
    }
}

struct WorkflowRowCells {
    id: String,
    status: String,
    start_time: String,
    runtime: String,
    running: String,
}

fn workflow_row_cells(
    id: &str,
    status: &str,
    start_time: &str,
    runtime: &str,
    running: &str,
) -> WorkflowRowCells {
    WorkflowRowCells {
        id: format!("{id:<name_width$}", name_width = NAME_WIDTH),
        status: format!("{status:<status_width$}", status_width = STATUS_WIDTH),
        start_time: format!("{start_time:<start_width$}", start_width = START_WIDTH),
        runtime: format!("{runtime:<runtime_width$}", runtime_width = RUNTIME_WIDTH),
        running: format!("{running:<running_width$}", running_width = RUNNING_WIDTH),
    }
}

fn workflow_row_prefix(cells: &WorkflowRowCells) -> String {
    format!(
        "{:<name_width$} {:<status_width$} {:<start_width$} {:<runtime_width$} {:<running_width$} ",
        cells.id,
        cells.status,
        cells.start_time,
        cells.runtime,
        cells.running,
        name_width = NAME_WIDTH,
        status_width = STATUS_WIDTH,
        start_width = START_WIDTH,
        runtime_width = RUNTIME_WIDTH,
        running_width = RUNNING_WIDTH,
    )
}

fn colored_workflow_row_prefix(
    cells: &WorkflowRowCells,
    status: &Status,
    running_display: &str,
    running_tasks: &[String],
) -> String {
    let running_column = if running_tasks.is_empty() {
        cells.running.clone()
    } else {
        format!(
            "{running_display:<running_width$}",
            running_width = RUNNING_WIDTH
        )
        .bright_blue()
        .to_string()
    };

    format!(
        "{} {} {} {} {} ",
        cells.id.bright_cyan(),
        color_status(&cells.status, status),
        cells.start_time.bright_black(),
        cells.runtime.bright_magenta(),
        running_column,
    )
}

fn format_start_time(status_log: &TaskStatusLog) -> String {
    status_log
        .start_time
        .map(|start| format_datetime(start))
        .unwrap_or_else(|| "-".into())
}

fn format_datetime(time: DateTime<Utc>) -> String {
    time.format("%Y-%m-%d %H:%M:%SZ").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn running_tasks_are_clamped() {
        let names = vec!["alpha".into(), "beta".into(), "gamma".into()];
        let display = running_tasks_display(&names);
        assert!(display.len() <= RUNNING_WIDTH);
    }

    #[test]
    fn workflow_prompt_prefers_non_empty_prompt() {
        let now = Utc::now();
        let mut summary = WorkflowSummary {
            id: "wf-1".into(),
            prompt: Some("boom".into()),
            notes: Some("note".into()),
            status: Status::Failed,
            status_log: TaskStatusLog {
                creation_time: now,
                start_time: None,
                end_time: None,
                current_status: Status::Failed,
            },
            running_tasks: vec![],
        };

        assert_eq!(workflow_prompt(&summary), "boom");

        summary.prompt = Some("   ".into());
        assert_eq!(workflow_prompt(&summary), "-");

        summary.prompt = None;
        assert_eq!(workflow_prompt(&summary), "-");
    }

    #[test]
    fn header_rows_only_include_prompt_column() {
        let prefix_width =
            workflow_row_prefix(&workflow_row_cells("ID", "STATUS", "STARTED", "RUNTIME", "RUNNING"))
                .len();
        let (plain, colored) = header_rows(prefix_width + TEXT_COLUMN_WIDTH);
        assert_eq!(plain.len(), colored.len());

        let first_line = plain.first().expect("header line");
        assert!(first_line.contains("PROMPT"));
        assert!(!first_line.contains("NOTES"));
    }

    #[test]
    fn format_workflow_lines_wraps_prompt_only() {
        let cells = workflow_row_cells("wf-2", "running", "10s", "10s", "task");
        let prefix = workflow_row_prefix(&cells);
        let lines =
            format_workflow_lines(&prefix, "long prompt content that should wrap cleanly", 120);

        assert!(!lines.is_empty());
        assert!(lines[0].starts_with(&prefix));
        assert!(lines.iter().all(|line| line.len() <= 120));
    }

    #[test]
    fn render_workflows_displays_prompt_without_notes() {
        let now = Utc::now();
        let workflows = vec![WorkflowSummary {
            id: "wf-3".into(),
            prompt: Some("the prompt to show".into()),
            notes: Some("notes should be hidden".into()),
            status: Status::Running,
            status_log: TaskStatusLog {
                creation_time: now,
                start_time: Some(now),
                end_time: None,
                current_status: Status::Running,
            },
            running_tasks: vec!["task-1".into()],
        }];

        let prefix_width = workflow_row_prefix(&workflow_row_cells(
            "wf-3",
            "STATUS",
            "STARTED",
            "RUNTIME",
            "RUNNING",
        ))
        .len();
        let lines = render_workflows(&workflows, prefix_width + TEXT_COLUMN_WIDTH, now);
        let combined = lines.join("\n");

        assert!(combined.contains("the prompt to show"));
        assert!(!combined.contains("notes should be hidden"));
    }

    #[test]
    fn prompt_is_truncated_after_five_lines() {
        let cells = workflow_row_cells("wf-4", "running", "start", "10s", "task");
        let prefix = workflow_row_prefix(&cells);
        let prompt = "chunk ".repeat(120);

        let lines = format_workflow_lines(&prefix, &prompt, prefix.len() + 20);

        assert_eq!(lines.len(), MAX_PROMPT_LINES);
        assert!(lines.last().unwrap().contains("..."));
    }

    #[test]
    fn format_start_time_uses_timestamp_when_present() {
        let start_time = Utc.with_ymd_and_hms(2024, 1, 2, 3, 4, 5).unwrap();
        let status_log = TaskStatusLog {
            creation_time: start_time,
            start_time: Some(start_time),
            end_time: None,
            current_status: Status::Running,
        };

        assert_eq!(format_start_time(&status_log), "2024-01-02 03:04:05Z");
    }

    #[test]
    fn format_start_time_returns_dash_when_missing() {
        let creation_time = Utc.with_ymd_and_hms(2024, 1, 2, 3, 4, 5).unwrap();
        let status_log = TaskStatusLog {
            creation_time,
            start_time: None,
            end_time: None,
            current_status: Status::Pending,
        };

        assert_eq!(format_start_time(&status_log), "-");
    }
}
