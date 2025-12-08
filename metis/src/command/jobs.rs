use crate::client::MetisClientInterface;
use anyhow::Result;
use textwrap::{termwidth, Options, WrapAlgorithm};

const NAME_WIDTH: usize = 48;
const STATUS_WIDTH: usize = 9;
const RUNTIME_WIDTH: usize = 12;
const DEFAULT_TERMINAL_WIDTH: usize = 80;

pub async fn run(client: &dyn MetisClientInterface) -> Result<()> {
    let response = client.list_jobs().await?;
    let terminal_width = current_terminal_width();

    if response.jobs.is_empty() {
        println!("No Metis jobs found.");
        return Ok(());
    }

    let header = format!(
        "{:<name_width$} {:<status_width$} {:<runtime_width$} {}",
        "ID",
        "STATUS",
        "RUNTIME",
        "NOTES",
        name_width = NAME_WIDTH,
        status_width = STATUS_WIDTH,
        runtime_width = RUNTIME_WIDTH
    );
    println!("{header}");
    println!("{}", "-".repeat(header.len()));

    for job in response.jobs {
        let runtime = job.runtime.unwrap_or_else(|| "-".into());
        let notes = job.notes.unwrap_or_else(|| "-".into());
        for line in format_job_lines(&job.id, &job.status, &runtime, &notes, terminal_width) {
            println!("{line}");
        }
    }

    Ok(())
}

fn format_job_lines(
    id: &str,
    status: &str,
    runtime: &str,
    notes: &str,
    terminal_width: usize,
) -> Vec<String> {
    let prefix = job_row_prefix(id, status, runtime);
    let indent = " ".repeat(prefix.len());
    let available_width = terminal_width.saturating_sub(prefix.len()).max(1);
    let wrapped_notes = textwrap::wrap(
        notes,
        Options::new(available_width)
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

fn job_row_prefix(id: &str, status: &str, runtime: &str) -> String {
    format!(
        "{:<name_width$} {:<status_width$} {:<runtime_width$} ",
        id,
        status,
        runtime,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_notes_to_terminal_width_and_indents_followup_lines() {
        let terminal_width = 72;
        let notes =
            "This is a long note that should wrap to the next line when it exceeds the terminal width.";

        let lines = format_job_lines("job-123", "running", "12s", notes, terminal_width);

        assert_eq!(lines.len(), 2);
        let expected_prefix = job_row_prefix("job-123", "running", "12s");
        assert!(lines[0].starts_with(&expected_prefix));
        assert!(lines[1].starts_with(&" ".repeat(expected_prefix.len())));
        assert!(lines[0].contains("This is a long note that should wrap"));
        assert!(lines[1].contains("exceeds the terminal width."));
    }
}
