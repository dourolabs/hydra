use crate::{
    client::MetisClientInterface,
    command::jobs::{
        color_status, current_terminal_width, format_job_lines, format_runtime,
        format_status_with_finished,
    },
};
use anyhow::Result;
use chrono::Utc;
use metis_common::{
    task_status::Status,
    workflows::WorkflowSummary,
};
use owo_colors::OwoColorize;

#[cfg(test)]
use metis_common::task_status::TaskStatusLog;

const NAME_WIDTH: usize = 36;
const STATUS_WIDTH: usize = 26;
const RUNTIME_WIDTH: usize = 12;
const RUNNING_WIDTH: usize = 18;

pub async fn run(client: &dyn MetisClientInterface) -> Result<()> {
    let response = client.list_workflows().await?;
    let terminal_width = current_terminal_width();
    let now = Utc::now();

    if response.workflows.is_empty() {
        println!("No Metis workflows found.");
        return Ok(());
    }

    let (plain_header, colored_header) = header_row();
    println!("{colored_header}");
    println!("{}", "-".repeat(plain_header.len()));

    for workflow in response.workflows {
        let status_display = format_status_with_finished(&workflow.status_log, now);
        let runtime = format_runtime(&workflow.status_log, now).unwrap_or_else(|| "-".into());
        let running = running_tasks_display(&workflow.running_tasks);
        let notes = workflow_note(&workflow).unwrap_or_else(|| "-".into());

        let cells = workflow_row_cells(&workflow.id, &status_display, &runtime, &running);
        let plain_prefix = workflow_row_prefix(&cells);
        let colored_prefix = colored_workflow_row_prefix(
            &cells,
            &workflow.status,
            &running,
            &workflow.running_tasks,
        );
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

    Ok(())
}

fn header_row() -> (String, String) {
    let cells = workflow_row_cells("ID", "STATUS", "RUNTIME", "RUNNING");
    let plain = format!(
        "{} {} {} {} {}",
        cells.id, cells.status, cells.runtime, cells.running, "NOTES"
    );
    let colored = format!(
        "{} {} {} {} {}",
        cells.id.bold(),
        cells.status.bold(),
        cells.runtime.bold(),
        cells.running.bold(),
        "NOTES".bold()
    );
    (plain, colored)
}

fn workflow_note(workflow: &WorkflowSummary) -> Option<String> {
    workflow.notes.clone()
}

fn running_tasks_display(running_tasks: &[String]) -> String {
    if running_tasks.is_empty() {
        "-".to_string()
    } else {
        let joined = running_tasks.join(", ");
        clamp_text(&joined, RUNNING_WIDTH)
    }
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
    runtime: String,
    running: String,
}

fn workflow_row_cells(id: &str, status: &str, runtime: &str, running: &str) -> WorkflowRowCells {
    WorkflowRowCells {
        id: format!("{id:<name_width$}", name_width = NAME_WIDTH),
        status: format!("{status:<status_width$}", status_width = STATUS_WIDTH),
        runtime: format!("{runtime:<runtime_width$}", runtime_width = RUNTIME_WIDTH),
        running: format!("{running:<running_width$}", running_width = RUNNING_WIDTH),
    }
}

fn workflow_row_prefix(cells: &WorkflowRowCells) -> String {
    format!(
        "{:<name_width$} {:<status_width$} {:<runtime_width$} {:<running_width$} ",
        cells.id,
        cells.status,
        cells.runtime,
        cells.running,
        name_width = NAME_WIDTH,
        status_width = STATUS_WIDTH,
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
        "{} {} {} {} ",
        cells.id.bright_cyan(),
        color_status(&cells.status, status),
        cells.runtime.bright_magenta(),
        running_column,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_tasks_are_clamped() {
        let names = vec!["alpha".into(), "beta".into(), "gamma".into()];
        let display = running_tasks_display(&names);
        assert!(display.len() <= RUNNING_WIDTH);
    }

    #[test]
    fn notes_use_workflow_notes_only() {
        let summary = WorkflowSummary {
            id: "wf-1".into(),
            notes: Some("boom".into()),
            status: Status::Failed,
            status_log: TaskStatusLog {
                creation_time: Utc::now(),
                start_time: None,
                end_time: None,
                current_status: Status::Failed,
            },
            running_tasks: vec![],
        };

        assert_eq!(workflow_note(&summary), Some("boom".into()));
    }
}
