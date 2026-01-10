use std::{
    cmp::Ordering,
    io::{stdout, Stdout},
    ops::ControlFlow,
    time::Duration,
};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use crossterm::{
    event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use metis_common::{
    artifacts::{Artifact, ArtifactKind, ArtifactRecord, SearchArtifactsQuery},
    jobs::JobSummary,
    task_status::{Status, TaskError, TaskStatusLog},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Table},
    Frame, Terminal,
};

use crate::{client::MetisClientInterface, command::jobs};

const JOB_REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const ARTIFACT_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const MAX_RUNNING_JOBS: usize = 10;
const MAX_FINISHED_JOBS: usize = 12;
const MAX_ARTIFACT_ROWS: usize = 12;
const MAX_MESSAGE_WIDTH: usize = 90;

#[derive(Default, Clone, PartialEq)]
struct JobSections {
    running: Vec<JobDisplay>,
    finished: Vec<JobDisplay>,
}

#[derive(Clone, PartialEq)]
struct JobDisplay {
    id: String,
    status: Status,
    runtime: Option<String>,
    note: String,
    last_change: Option<DateTime<Utc>>,
}

#[derive(Default, Clone, PartialEq)]
struct ArtifactSections {
    issues: Vec<ArtifactDisplay>,
    patches: Vec<ArtifactDisplay>,
}

#[derive(Clone, PartialEq)]
struct ArtifactDisplay {
    id: String,
    summary: String,
}

#[derive(Default, Clone, PartialEq)]
struct DashboardState {
    jobs: JobSections,
    artifacts: ArtifactSections,
    jobs_error: Option<String>,
    artifacts_error: Option<String>,
}

pub async fn run(client: &dyn MetisClientInterface) -> Result<()> {
    let mut terminal = setup_terminal()?;
    let dashboard_result = run_dashboard_loop(client, &mut terminal).await;
    teardown_terminal(&mut terminal)?;
    dashboard_result
}

async fn run_dashboard_loop(
    client: &dyn MetisClientInterface,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
) -> Result<()> {
    let mut state = DashboardState::default();
    let mut needs_draw = true;

    match refresh_jobs(client, &mut state).await {
        Ok(changed) => needs_draw |= changed,
        Err(err) => {
            state.jobs_error = Some(format!("Failed to load jobs: {err}"));
            needs_draw = true;
        }
    }

    match refresh_artifacts(client, &mut state).await {
        Ok(changed) => needs_draw |= changed,
        Err(err) => {
            state.artifacts_error = Some(format!("Failed to load artifacts: {err}"));
            needs_draw = true;
        }
    }

    if needs_draw {
        terminal.draw(|f| render(f, &state))?;
        needs_draw = false;
    }

    let mut events = EventStream::new();
    let mut jobs_tick = tokio::time::interval(JOB_REFRESH_INTERVAL);
    let mut artifacts_tick = tokio::time::interval(ARTIFACT_REFRESH_INTERVAL);

    loop {
        tokio::select! {
            _ = jobs_tick.tick() => {
                match refresh_jobs(client, &mut state).await {
                    Ok(changed) => {
                        state.jobs_error = None;
                        needs_draw |= changed;
                    }
                    Err(err) => {
                        state.jobs_error = Some(format!("Failed to refresh jobs: {err}"));
                        needs_draw = true;
                    }
                }
            }
            _ = artifacts_tick.tick() => {
                match refresh_artifacts(client, &mut state).await {
                    Ok(changed) => {
                        state.artifacts_error = None;
                        needs_draw |= changed;
                    }
                    Err(err) => {
                        state.artifacts_error = Some(format!("Failed to refresh artifacts: {err}"));
                        needs_draw = true;
                    }
                }
            }
            maybe_event = events.next() => {
                match maybe_event {
                    Some(Ok(event)) => {
                        if matches!(handle_event(event), ControlFlow::Break(())) {
                            break;
                        }
                        needs_draw = true;
                    }
                    Some(Err(err)) => {
                        state.jobs_error = Some(format!("Event stream error: {err}"));
                        needs_draw = true;
                    }
                    None => break,
                }
            }
        }

        if needs_draw {
            terminal.draw(|f| render(f, &state))?;
            needs_draw = false;
        }
    }

    Ok(())
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode().context("failed to enable raw mode")?;
    execute!(stdout(), EnterAlternateScreen).context("failed to switch to alternate screen")?;
    let backend = CrosstermBackend::new(stdout());
    Terminal::new(backend).context("failed to initialize terminal")
}

fn teardown_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode().context("failed to disable raw mode")?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .context("failed to leave alternate screen")?;
    terminal.show_cursor().context("failed to show cursor")
}

fn handle_event(event: Event) -> ControlFlow<()> {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => ControlFlow::Break(()),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                ControlFlow::Break(())
            }
            _ => ControlFlow::Continue(()),
        },
        Event::Resize(_, _) => ControlFlow::Continue(()),
        _ => ControlFlow::Continue(()),
    }
}

fn render(frame: &mut Frame, state: &DashboardState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(10),
            Constraint::Length(9),
        ])
        .split(frame.size());

    render_header(frame, chunks[0], state);
    render_jobs(frame, chunks[1], state);
    render_artifacts(frame, chunks[2], state);
}

fn render_header(frame: &mut Frame, area: ratatui::layout::Rect, state: &DashboardState) {
    let mut lines = vec![Line::from(vec![
        Span::styled(
            "Metis Dashboard",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(" — press q or Esc to exit."),
    ])];

    if let Some(error) = &state.jobs_error {
        lines.push(Line::from(Span::styled(
            format!("Jobs: {error}"),
            Style::default().fg(Color::Red),
        )));
    }

    if let Some(error) = &state.artifacts_error {
        lines.push(Line::from(Span::styled(
            format!("Artifacts: {error}"),
            Style::default().fg(Color::Red),
        )));
    }

    let paragraph = Paragraph::new(lines).block(
        Block::default()
            .title("Status")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::White)),
    );
    frame.render_widget(paragraph, area);
}

fn render_jobs(frame: &mut Frame, area: ratatui::layout::Rect, state: &DashboardState) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let running_table = job_table(
        "Running jobs",
        &state.jobs.running,
        &[
            Constraint::Percentage(28),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Min(10),
        ],
    );
    let finished_table = job_table(
        "Recent results",
        &state.jobs.finished,
        &[
            Constraint::Percentage(28),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Min(10),
        ],
    );

    frame.render_widget(running_table, columns[0]);
    frame.render_widget(finished_table, columns[1]);
}

fn job_table<'a>(title: &'a str, jobs: &'a [JobDisplay], widths: &'a [Constraint]) -> Table<'a> {
    let header = Row::new(vec![
        Cell::from("ID").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Status").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Runtime").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Notes / Errors").style(Style::default().add_modifier(Modifier::BOLD)),
    ]);

    let rows: Vec<Row> = if jobs.is_empty() {
        vec![Row::new(vec![
            Cell::from("No data").style(Style::default().fg(Color::DarkGray)),
            Cell::default(),
            Cell::default(),
            Cell::default(),
        ])]
    } else {
        jobs.iter()
            .map(|job| {
                Row::new(vec![
                    Cell::from(job.id.clone()),
                    Cell::from(status_label(job.status)).style(status_style(job.status)),
                    Cell::from(job.runtime.clone().unwrap_or_else(|| "-".into())),
                    Cell::from(truncate_message(&job.note, MAX_MESSAGE_WIDTH)),
                ])
            })
            .collect()
    };

    Table::new(rows, widths)
        .header(header)
        .block(Block::default().title(title).borders(Borders::ALL))
}

fn render_artifacts(frame: &mut Frame, area: ratatui::layout::Rect, state: &DashboardState) {
    let rows = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let issues = artifact_list("Recent issues", &state.artifacts.issues, Color::Yellow);
    let patches = artifact_list("Recent patches", &state.artifacts.patches, Color::Cyan);

    frame.render_widget(issues, rows[0]);
    frame.render_widget(patches, rows[1]);
}

fn artifact_list<'a>(title: &'a str, artifacts: &'a [ArtifactDisplay], color: Color) -> List<'a> {
    let items: Vec<ListItem> = if artifacts.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "No records",
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        artifacts
            .iter()
            .map(|artifact| {
                ListItem::new(Line::from(vec![
                    Span::styled(&artifact.id, Style::default().fg(color)),
                    Span::raw(" — "),
                    Span::raw(truncate_message(&artifact.summary, MAX_MESSAGE_WIDTH)),
                ]))
            })
            .collect()
    };

    List::new(items).block(Block::default().title(title).borders(Borders::ALL))
}

async fn refresh_jobs(
    client: &dyn MetisClientInterface,
    state: &mut DashboardState,
) -> Result<bool> {
    let response = client.list_jobs().await?;
    let now = Utc::now();
    let jobs = categorize_jobs(response.jobs, now);

    if state.jobs == jobs {
        return Ok(false);
    }

    state.jobs = jobs;
    Ok(true)
}

async fn refresh_artifacts(
    client: &dyn MetisClientInterface,
    state: &mut DashboardState,
) -> Result<bool> {
    let issues = fetch_artifacts(client, ArtifactKind::Issue).await?;
    let patches = fetch_artifacts(client, ArtifactKind::Patch).await?;
    let artifacts = ArtifactSections { issues, patches };

    if state.artifacts == artifacts {
        return Ok(false);
    }

    state.artifacts = artifacts;
    Ok(true)
}

async fn fetch_artifacts(
    client: &dyn MetisClientInterface,
    kind: ArtifactKind,
) -> Result<Vec<ArtifactDisplay>> {
    let response = client
        .list_artifacts(&SearchArtifactsQuery {
            artifact_type: Some(kind),
            issue_type: None,
            q: None,
        })
        .await
        .with_context(|| format!("failed to fetch {kind:?} artifacts"))?;

    let mut artifacts: Vec<ArtifactDisplay> = response
        .artifacts
        .into_iter()
        .filter_map(|record| artifact_to_display(record, kind))
        .collect();

    artifacts.truncate(MAX_ARTIFACT_ROWS);
    Ok(artifacts)
}

fn artifact_to_display(
    record: ArtifactRecord,
    expected_kind: ArtifactKind,
) -> Option<ArtifactDisplay> {
    match (record.artifact, expected_kind) {
        (Artifact::Issue { description, .. }, ArtifactKind::Issue) => Some(ArtifactDisplay {
            id: record.id,
            summary: description,
        }),
        (Artifact::Patch { description, .. }, ArtifactKind::Patch) => Some(ArtifactDisplay {
            id: record.id,
            summary: description,
        }),
        _ => None,
    }
}

fn categorize_jobs(jobs: Vec<JobSummary>, now: DateTime<Utc>) -> JobSections {
    let mut running = Vec::new();
    let mut finished = Vec::new();

    for job in jobs {
        let display = summarize_job(job, now);
        match display.status {
            Status::Complete | Status::Failed => finished.push(display),
            _ => running.push(display),
        }
    }

    running.sort_by(|a, b| compare_recent(a.last_change, b.last_change));
    finished.sort_by(|a, b| compare_recent(a.last_change, b.last_change));

    if running.len() > MAX_RUNNING_JOBS {
        running.truncate(MAX_RUNNING_JOBS);
    }
    if finished.len() > MAX_FINISHED_JOBS {
        finished.truncate(MAX_FINISHED_JOBS);
    }

    JobSections { running, finished }
}

fn summarize_job(job: JobSummary, now: DateTime<Utc>) -> JobDisplay {
    let status = job.status_log.current_status();
    let runtime = jobs::format_runtime(&job.status_log, now);
    let last_change = last_activity(&job.status_log);
    let note = note_or_error(&job);

    JobDisplay {
        id: job.id,
        status,
        runtime,
        note,
        last_change,
    }
}

fn last_activity(status_log: &TaskStatusLog) -> Option<DateTime<Utc>> {
    status_log
        .end_time()
        .or_else(|| status_log.start_time())
        .or_else(|| status_log.creation_time())
}

fn compare_recent(a: Option<DateTime<Utc>>, b: Option<DateTime<Utc>>) -> Ordering {
    match (a, b) {
        (Some(left), Some(right)) => right.cmp(&left),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn note_or_error(job: &JobSummary) -> String {
    if let Some(Err(error)) = job.status_log.result() {
        return format_task_error(&error);
    }

    if let Some(notes) = &job.notes {
        let trimmed = notes.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    "-".into()
}

fn format_task_error(error: &TaskError) -> String {
    match error {
        TaskError::JobEngineError { reason } => format!("error: {reason}"),
    }
}

fn status_label(status: Status) -> &'static str {
    jobs::format_status(&status)
}

fn status_style(status: Status) -> Style {
    match status {
        Status::Complete => Style::default().fg(Color::Green),
        Status::Running => Style::default().fg(Color::Yellow),
        Status::Failed => Style::default().fg(Color::Red),
        Status::Blocked => Style::default().fg(Color::Magenta),
        Status::Pending => Style::default().fg(Color::Blue),
    }
}

fn truncate_message(message: &str, max_chars: usize) -> String {
    if message.chars().count() <= max_chars {
        return message.to_string();
    }

    message
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>()
        + "..."
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration as ChronoDuration;
    use metis_common::task_status::Event;

    fn job_with_status(id: &str, status: Status, offset_seconds: i64) -> JobSummary {
        let now = Utc::now() - ChronoDuration::seconds(offset_seconds);
        let mut log = TaskStatusLog::new(Status::Pending, now);
        match status {
            Status::Running => log.events.push(Event::Started { at: now }),
            Status::Complete => {
                log.events.push(Event::Started { at: now });
                log.events.push(Event::Completed { at: now });
            }
            Status::Failed => {
                log.events.push(Event::Started { at: now });
                log.events.push(Event::Failed {
                    at: now,
                    error: TaskError::JobEngineError {
                        reason: "boom".into(),
                    },
                });
            }
            Status::Blocked | Status::Pending => {}
        }

        JobSummary {
            id: id.to_string(),
            notes: None,
            program: "0".into(),
            params: vec![],
            status_log: log,
        }
    }

    #[test]
    fn categorize_jobs_splits_running_and_finished() {
        let jobs = vec![
            job_with_status("job-running", Status::Running, 0),
            job_with_status("job-complete", Status::Complete, 1),
            job_with_status("job-failed", Status::Failed, 2),
        ];

        let categorized = categorize_jobs(jobs, Utc::now());

        assert_eq!(categorized.running.len(), 1);
        assert_eq!(categorized.finished.len(), 2);
        assert_eq!(categorized.running[0].id, "job-running");
        assert!(categorized
            .finished
            .iter()
            .any(|job| job.id == "job-complete"));
        assert!(categorized
            .finished
            .iter()
            .any(|job| job.id == "job-failed"));
    }

    #[test]
    fn note_or_error_prefers_error_reason() {
        let mut job = job_with_status("job-failed", Status::Failed, 0);
        job.notes = Some("note that should be ignored".into());

        let message = note_or_error(&job);

        assert!(message.contains("boom"));
        assert!(!message.contains("note that should be ignored"));
    }

    #[test]
    fn truncate_message_limits_length() {
        let long = "a".repeat(120);
        let truncated = truncate_message(&long, 20);

        assert_eq!(truncated.len(), 20);
        assert!(truncated.ends_with("..."));
    }
}
