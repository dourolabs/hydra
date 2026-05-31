use std::io::Write;

use anyhow::Result;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use hydra_common::{
    api::v1::sessions::TokenUsage,
    sessions::{Session, SessionSummary, SessionSummaryRecord, SessionVersionRecord},
    task_status::{Status, TaskError},
};
use owo_colors::OwoColorize;
use textwrap::{termwidth, Options, WrapAlgorithm};

use crate::util::{format_duration, format_thousands, truncate_lines};

use super::Render;

const NAME_WIDTH: usize = 48;
const STATUS_WIDTH: usize = 26;
const RUNTIME_WIDTH: usize = 12;
const MAX_NOTES_WIDTH: usize = 80;
const MAX_NOTE_LINES: usize = 5;
const DEFAULT_TERMINAL_WIDTH: usize = 80;

pub struct SessionRecords<'a>(pub &'a [SessionVersionRecord]);

pub struct SessionSummaryRecords<'a>(pub &'a [SessionSummaryRecord]);

impl Render for SessionRecords<'_> {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        for job in self.0 {
            serde_json::to_writer(&mut *writer, job)?;
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        if self.0.is_empty() {
            writeln!(writer, "No Hydra sessions found.")?;
            writer.flush()?;
            return Ok(());
        }

        let terminal_width = current_terminal_width();

        let (plain_header, colored_header) = header_row();
        writeln!(writer, "{colored_header}")?;
        writeln!(writer, "{}", "-".repeat(plain_header.len()))?;

        let now = Utc::now();
        for job in self.0 {
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
}

impl Render for SessionSummaryRecords<'_> {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        for job in self.0 {
            serde_json::to_writer(&mut *writer, job)?;
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        if self.0.is_empty() {
            writeln!(writer, "No Hydra sessions found.")?;
            writer.flush()?;
            return Ok(());
        }

        let terminal_width = current_terminal_width();

        let (plain_header, colored_header) = header_row();
        writeln!(writer, "{colored_header}")?;
        writeln!(writer, "{}", "-".repeat(plain_header.len()))?;

        let now = Utc::now();
        for job in self.0 {
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
    let mut parts: Vec<String> = Vec::new();
    if let Some(error) = job.session.error.as_ref() {
        parts.push(format_task_error(error));
    }
    parts.push(format_token_usage_segment(job.session.usage.as_ref()));
    Some(parts.join(" | "))
}

fn session_summary_note(job: &SessionSummaryRecord) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(error) = job.session.error.as_ref() {
        parts.push(format_task_error(error));
    }
    if let Some(conversation_id) = job.session.conversation_id.as_ref() {
        parts.push(format!("conversation: {conversation_id}"));
    }
    parts.push(format_token_usage_segment(job.session.usage.as_ref()));
    Some(parts.join(" | "))
}

fn format_token_usage_segment(usage: Option<&TokenUsage>) -> String {
    let Some(usage) = usage else {
        return "tokens: —".to_string();
    };
    let total = usage
        .input_tokens
        .saturating_add(usage.output_tokens)
        .saturating_add(usage.cache_read_input_tokens)
        .saturating_add(usage.cache_creation_input_tokens);
    format!(
        "tokens: total={} | input={} out={} cache_read={} cache_create={}",
        format_thousands(total),
        format_thousands(usage.input_tokens),
        format_thousands(usage.output_tokens),
        format_thousands(usage.cache_read_input_tokens),
        format_thousands(usage.cache_creation_input_tokens),
    )
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
            let started = start_time.or(creation_time)?;
            let duration = if now < started {
                ChronoDuration::zero()
            } else {
                now - started
            };
            Some(format_duration(duration))
        }
        Status::Pending | Status::Created => {
            let created = creation_time?;
            let duration = if now < created {
                ChronoDuration::zero()
            } else {
                now - created
            };
            Some(format_duration(duration))
        }
        Status::Complete | Status::Failed => {
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use hydra_common::sessions::Session;
    use hydra_common::SessionId;

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

    fn build_session(
        conversation_id: Option<hydra_common::ConversationId>,
        usage: Option<hydra_common::api::v1::sessions::TokenUsage>,
    ) -> Session {
        use hydra_common::api::v1::sessions::{
            AgentConfig, MountItem, MountSpec, RelativePath, SessionMode,
        };
        let mode = match conversation_id {
            Some(id) => SessionMode::Interactive {
                conversation_id: id,
                idle_timeout_secs: None,
                greet_user: false,
            },
            None => SessionMode::Headless,
        };
        let mount_spec = MountSpec::new(
            RelativePath::new("repo").unwrap(),
            vec![MountItem::Documents {
                target: RelativePath::new("documents").unwrap(),
            }],
        );
        let mut session = Session::new(
            "alice".into(),
            None,
            None,
            AgentConfig::default(),
            mount_spec,
            None,
            std::collections::HashMap::new(),
            None,
            None,
            None,
            mode,
            Status::Complete,
            None,
            None,
            false,
            None,
            None,
            None,
        );
        session.usage = usage;
        session
    }

    fn build_summary_record(
        conversation_id: Option<hydra_common::ConversationId>,
        usage: Option<hydra_common::api::v1::sessions::TokenUsage>,
    ) -> SessionSummaryRecord {
        let session = build_session(conversation_id, usage);
        let summary = SessionSummary::from(&session);
        let json = serde_json::json!({
            "session_id": SessionId::new(),
            "version": 1u64,
            "timestamp": Utc::now(),
            "session": serde_json::to_value(&summary).unwrap(),
        });
        serde_json::from_value(json).unwrap()
    }

    fn build_version_record(
        usage: Option<hydra_common::api::v1::sessions::TokenUsage>,
    ) -> SessionVersionRecord {
        let session = build_session(None, usage);
        let json = serde_json::json!({
            "session_id": SessionId::new(),
            "version": 1u64,
            "timestamp": Utc::now(),
            "session": serde_json::to_value(&session).unwrap(),
        });
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn session_summary_note_renders_tokens_and_combines_with_other_notes() {
        use hydra_common::api::v1::sessions::TokenUsage;
        use hydra_common::ConversationId;

        let conv_id = ConversationId::new();
        let record = build_summary_record(
            Some(conv_id.clone()),
            Some(TokenUsage {
                input_tokens: 1_000,
                output_tokens: 2_000,
                cache_read_input_tokens: 3_000,
                cache_creation_input_tokens: 4_000,
            }),
        );

        let note = session_summary_note(&record).expect("note present");
        assert!(
            note.contains(&format!("conversation: {conv_id}")),
            "expected conversation in {note}"
        );
        assert!(
            note.contains(
                "tokens: total=10,000 | input=1,000 out=2,000 cache_read=3,000 cache_create=4,000"
            ),
            "expected new-shape tokens in {note}"
        );
        assert!(note.contains(" | "), "expected separator in {note}");
    }

    #[test]
    fn session_note_renders_tokens_when_usage_present() {
        use hydra_common::api::v1::sessions::TokenUsage;

        let record = build_version_record(Some(TokenUsage {
            input_tokens: 33_947_111,
            output_tokens: 250_000,
            cache_read_input_tokens: 1_500_000,
            cache_creation_input_tokens: 50_000,
        }));

        let note = session_note(&record).expect("note present");
        assert!(
            note.contains("tokens: total=35,747,111 | input=33,947,111 out=250,000 cache_read=1,500,000 cache_create=50,000"),
            "expected new-shape tokens in {note}"
        );
    }
}
