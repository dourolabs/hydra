use crate::client::MetisClientInterface;
use anyhow::Result;
use owo_colors::OwoColorize;
use textwrap::{termwidth, Options, WrapAlgorithm};

const NAME_WIDTH: usize = 48;
const STATUS_WIDTH: usize = 9;
const RUNTIME_WIDTH: usize = 12;
const MAX_NOTES_WIDTH: usize = 80;
const DEFAULT_TERMINAL_WIDTH: usize = 80;

pub async fn run(client: &dyn MetisClientInterface) -> Result<()> {
    let response = client.list_jobs().await?;
    let terminal_width = current_terminal_width();

    if response.jobs.is_empty() {
        println!("No Metis jobs found.");
        return Ok(());
    }

    let (plain_header, colored_header) = header_row();
    println!("{colored_header}");
    println!("{}", "-".repeat(plain_header.len()));

    for job in response.jobs {
        let runtime = job.runtime.unwrap_or_else(|| "-".into());
        let notes = job.notes.unwrap_or_else(|| "-".into());
        let cells = job_row_cells(&job.id, &job.status, &runtime);
        let plain_prefix = job_row_prefix(&cells);
        let colored_prefix = colored_job_row_prefix(&cells, &job.status);
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

fn format_job_lines(prefix: &str, notes: &str, terminal_width: usize) -> Vec<String> {
    let indent = " ".repeat(prefix.len());
    let available_width = terminal_width.saturating_sub(prefix.len()).max(1);
    let notes_width = available_width.min(MAX_NOTES_WIDTH);
    let wrapped_notes = textwrap::wrap(
        notes,
        Options::new(notes_width)
            .break_words(true)
            .wrap_algorithm(WrapAlgorithm::FirstFit),
    );

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
        id: format!("{id:<name_width$}", name_width = NAME_WIDTH),
        status: format!("{status:<status_width$}", status_width = STATUS_WIDTH),
        runtime: format!("{runtime:<runtime_width$}", runtime_width = RUNTIME_WIDTH),
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

fn colored_job_row_prefix(cells: &JobRowCells, status: &str) -> String {
    format!(
        "{} {} {} ",
        cells.id.bright_cyan(),
        color_status(&cells.status, status),
        cells.runtime.bright_magenta(),
    )
}

fn color_status(padded_status: &str, status: &str) -> String {
    if status.eq_ignore_ascii_case("complete") {
        padded_status.green().to_string()
    } else if status.eq_ignore_ascii_case("running") {
        padded_status.yellow().to_string()
    } else if status.eq_ignore_ascii_case("failed") {
        padded_status.red().to_string()
    } else {
        padded_status.bold().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
