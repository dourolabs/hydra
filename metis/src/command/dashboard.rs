use std::{
    cmp::Ordering,
    collections::{BTreeSet, HashMap, HashSet},
    io::{stdout, Stdout},
    time::Duration,
};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use crossterm::{
    event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use metis_common::{
    issues::{
        Issue, IssueDependency, IssueDependencyType, IssueRecord as ApiIssueRecord, IssueStatus,
        IssueType, SearchIssuesQuery, UpsertIssueRequest,
    },
    jobs::{JobRecord, SearchJobsQuery},
    task_status::{Status, TaskError, TaskStatusLog},
    IssueId, TaskId,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame, Terminal,
};

use crate::{client::MetisClientInterface, command::jobs};

const JOB_REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const RECORD_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const MAX_MESSAGE_WIDTH: usize = 90;
const ISSUE_ID_VAR: &str = "METIS_ISSUE_ID";

#[derive(Clone, PartialEq)]
struct JobDetails {
    display: JobDisplay,
    issue_id: Option<IssueId>,
}

#[derive(Clone, PartialEq)]
struct JobDisplay {
    id: TaskId,
    status: Status,
    runtime: Option<String>,
    note: String,
    last_change: Option<DateTime<Utc>>,
}

#[derive(Clone, PartialEq)]
struct IssueRecord {
    id: IssueId,
    description: String,
    progress: String,
    status: IssueStatus,
    assignee: Option<String>,
    dependencies: Vec<IssueDependency>,
}

#[derive(Default, Clone, PartialEq)]
struct IssueLines {
    rows: Vec<IssueLine>,
}

#[derive(Clone, PartialEq)]
struct IssueLine {
    id: String,
    summary: String,
    progress: Option<String>,
    status: IssueStatus,
    readiness: IssueReadiness,
    assignee: Option<String>,
    task: Option<TaskIndicator>,
    depth: usize,
}

#[derive(Clone, PartialEq, Debug)]
enum IssueReadiness {
    Ready,
    Blocked(Vec<String>),
    Waiting,
    Dropped,
}

#[derive(Clone)]
struct IssueNode {
    record: IssueRecord,
    parent: Option<IssueId>,
    children: Vec<IssueId>,
    task: Option<TaskIndicator>,
}

#[derive(Clone, PartialEq)]
struct TaskIndicator {
    status: Status,
    runtime: Option<String>,
}

struct IssueSummary {
    summary: String,
    progress: Option<String>,
}

#[derive(Clone, PartialEq)]
struct IssueDraft {
    prompt: String,
    assignees: Vec<String>,
    assignee_index: usize,
    validation_error: Option<String>,
    info_message: Option<String>,
    editing: bool,
    is_submitting: bool,
}

impl Default for IssueDraft {
    fn default() -> Self {
        Self {
            prompt: String::new(),
            assignees: vec!["pm".to_string()],
            assignee_index: 0,
            validation_error: None,
            info_message: None,
            editing: false,
            is_submitting: false,
        }
    }
}

impl IssueDraft {
    fn selected_assignee(&self) -> Option<&str> {
        self.assignees
            .get(self.assignee_index)
            .map(|assignee| assignee.as_str())
    }

    fn cycle_assignee(&mut self, forward: bool) {
        if self.assignees.is_empty() {
            return;
        }

        let total = self.assignees.len();
        let next = if forward {
            (self.assignee_index + 1) % total
        } else {
            self.assignee_index.saturating_add(total - 1) % total
        };
        self.assignee_index = next;
    }

    fn note_edit(&mut self) {
        self.validation_error = None;
        self.info_message = None;
    }
}

#[derive(Default, Clone, PartialEq)]
struct DashboardState {
    jobs: Vec<JobDetails>,
    issues: Vec<IssueRecord>,
    issue_lines: IssueLines,
    assigned_issue_lines: IssueLines,
    jobs_error: Option<String>,
    records_error: Option<String>,
    username: Option<String>,
    issue_draft: IssueDraft,
}

struct IssueSubmission {
    prompt: String,
    assignee: String,
}

struct EventOutcome {
    should_quit: bool,
    submission: Option<IssueSubmission>,
}

pub async fn run(client: &dyn MetisClientInterface, username: Option<String>) -> Result<()> {
    let mut terminal = setup_terminal()?;
    let dashboard_result = run_dashboard_loop(client, &mut terminal, username).await;
    teardown_terminal(&mut terminal)?;
    dashboard_result
}

async fn run_dashboard_loop(
    client: &dyn MetisClientInterface,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    username: Option<String>,
) -> Result<()> {
    let mut state = DashboardState {
        username,
        ..DashboardState::default()
    };
    let mut needs_draw = true;

    match refresh_jobs(client, &mut state).await {
        Ok(changed) => needs_draw |= changed,
        Err(err) => {
            state.jobs_error = Some(format!("Failed to load jobs: {err}"));
            needs_draw = true;
        }
    }

    match refresh_records(client, &mut state).await {
        Ok(changed) => needs_draw |= changed,
        Err(err) => {
            state.records_error = Some(format!("Failed to load issues: {err}"));
            needs_draw = true;
        }
    }

    if needs_draw {
        terminal.draw(|f| render(f, &state))?;
        needs_draw = false;
    }

    let mut events = EventStream::new();
    let mut jobs_tick = tokio::time::interval(JOB_REFRESH_INTERVAL);
    let mut records_tick = tokio::time::interval(RECORD_REFRESH_INTERVAL);

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
            _ = records_tick.tick() => {
                match refresh_records(client, &mut state).await {
                    Ok(changed) => {
                        state.records_error = None;
                        needs_draw |= changed;
                    }
                    Err(err) => {
                        state.records_error = Some(format!("Failed to refresh issues: {err}"));
                        needs_draw = true;
                    }
                }
            }
            maybe_event = events.next() => {
                match maybe_event {
                    Some(Ok(event)) => {
                        let outcome = handle_event(event, &mut state);
                        if outcome.should_quit {
                            break;
                        }
                        if let Some(submission) = outcome.submission {
                            let assignee = submission.assignee.clone();
                            state.issue_draft.info_message =
                                Some(format!("Submitting issue for @{assignee}..."));
                            terminal.draw(|f| render(f, &state))?;
                            let submission_result = submit_issue(client, &submission).await;
                            handle_issue_submission_result(
                                &mut state,
                                &assignee,
                                submission_result,
                            );
                            if state.issue_draft.validation_error.is_none() {
                                match refresh_records(client, &mut state).await {
                                    Ok(_changed) => {
                                        state.records_error = None;
                                    }
                                    Err(err) => {
                                        state.records_error =
                                            Some(format!("Failed to refresh issues: {err}"));
                                    }
                                }
                            }
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

fn handle_event(event: Event, state: &mut DashboardState) -> EventOutcome {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => EventOutcome {
                should_quit: true,
                submission: None,
            },
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => EventOutcome {
                should_quit: true,
                submission: None,
            },
            _ => EventOutcome {
                should_quit: false,
                submission: handle_issue_draft_key(key, state),
            },
        },
        Event::Resize(_, _) => EventOutcome {
            should_quit: false,
            submission: None,
        },
        _ => EventOutcome {
            should_quit: false,
            submission: None,
        },
    }
}

fn handle_issue_draft_key(key: KeyEvent, state: &mut DashboardState) -> Option<IssueSubmission> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('n') {
        state.issue_draft.editing = !state.issue_draft.editing;
        return None;
    }

    if state.issue_draft.is_submitting {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Enter {
            state.issue_draft.info_message =
                Some("Issue submission already in progress.".to_string());
        }
        return None;
    }

    match key.code {
        KeyCode::Tab => {
            state.issue_draft.cycle_assignee(true);
            return None;
        }
        KeyCode::BackTab => {
            state.issue_draft.cycle_assignee(false);
            return None;
        }
        _ => {}
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Enter {
        return attempt_issue_submit(state);
    }

    if !state.issue_draft.editing {
        return None;
    }

    match key.code {
        KeyCode::Char(c)
            if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT =>
        {
            state.issue_draft.prompt.push(c);
            state.issue_draft.note_edit();
        }
        KeyCode::Backspace => {
            state.issue_draft.prompt.pop();
            state.issue_draft.note_edit();
        }
        KeyCode::Enter => {
            state.issue_draft.prompt.push('\n');
            state.issue_draft.note_edit();
        }
        _ => {}
    }

    None
}

fn attempt_issue_submit(state: &mut DashboardState) -> Option<IssueSubmission> {
    if state.issue_draft.is_submitting {
        state.issue_draft.info_message = Some("Issue submission already in progress.".to_string());
        state.issue_draft.validation_error = None;
        return None;
    }

    let prompt = state.issue_draft.prompt.trim();
    let assignee = state
        .issue_draft
        .selected_assignee()
        .unwrap_or("pm")
        .to_string();

    if prompt.is_empty() {
        state.issue_draft.validation_error = Some("Prompt cannot be empty.".to_string());
        state.issue_draft.info_message = None;
        return None;
    }

    state.issue_draft.validation_error = None;
    state.issue_draft.info_message = None;
    state.issue_draft.is_submitting = true;

    Some(IssueSubmission {
        prompt: prompt.to_string(),
        assignee,
    })
}

fn handle_issue_submission_result(
    state: &mut DashboardState,
    assignee: &str,
    result: Result<IssueId>,
) {
    state.issue_draft.is_submitting = false;
    match result {
        Ok(issue_id) => {
            state.issue_draft.prompt.clear();
            state.issue_draft.editing = false;
            state.issue_draft.validation_error = None;
            state.issue_draft.info_message =
                Some(format!("Created issue {issue_id} for @{assignee}."));
        }
        Err(err) => {
            state.issue_draft.validation_error =
                Some(format!("Failed to create issue for @{assignee}: {err}"));
            state.issue_draft.info_message = None;
        }
    }
}

async fn submit_issue(
    client: &dyn MetisClientInterface,
    submission: &IssueSubmission,
) -> Result<IssueId> {
    let assignee = submission.assignee.trim();
    let assignee = if assignee.is_empty() {
        None
    } else {
        Some(assignee.to_string())
    };

    let request = UpsertIssueRequest {
        issue: Issue {
            issue_type: IssueType::Task,
            description: submission.prompt.trim().to_string(),
            progress: String::new(),
            status: IssueStatus::Open,
            assignee,
            dependencies: Vec::new(),
            patches: Vec::new(),
        },
        job_id: None,
    };

    let response = client
        .create_issue(&request)
        .await
        .context("failed to create issue")?;
    Ok(response.issue_id)
}

fn render(frame: &mut Frame, state: &DashboardState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(12),
            Constraint::Length(8),
        ])
        .split(frame.size());

    render_header(frame, chunks[0], state);
    render_issue_sections(frame, chunks[1], state);
    render_issue_creator(frame, chunks[2], state);
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

    if let Some(error) = &state.records_error {
        lines.push(Line::from(Span::styled(
            format!("Issues: {error}"),
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

fn render_issue_sections(frame: &mut Frame, area: ratatui::layout::Rect, state: &DashboardState) {
    let has_username = state
        .username
        .as_deref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);

    if has_username {
        let panels = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(area);

        let username = state.username.as_deref().unwrap();
        render_issue_list(
            frame,
            panels[0],
            &state.assigned_issue_lines,
            &format!("Open issues for @{username}"),
            &format!("No open issues assigned to @{username}"),
        );
        render_issue_list(
            frame,
            panels[1],
            &state.issue_lines,
            "Running issues",
            "No issues found",
        );
    } else {
        render_issue_list(
            frame,
            area,
            &state.issue_lines,
            "Running issues",
            "No issues found",
        );
    }
}

fn render_issue_creator(frame: &mut Frame, area: ratatui::layout::Rect, state: &DashboardState) {
    let draft = &state.issue_draft;
    let title = if draft.editing {
        "New issue (editing)"
    } else {
        "New issue"
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::White));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);

    let prompt_render = build_prompt_render(draft, sections[0]);
    let prompt = Paragraph::new(prompt_render.lines).wrap(Wrap { trim: false });
    frame.render_widget(prompt, sections[0]);
    if let Some((x, y)) = prompt_render.cursor {
        frame.set_cursor(x, y);
    }

    let assignee = draft.selected_assignee().unwrap_or("pm");
    let assignee_line = Line::from(vec![
        Span::styled("Assignee: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(format!("@{assignee}"), Style::default().fg(Color::Yellow)),
        Span::styled("  (Tab to change)", Style::default().fg(Color::DarkGray)),
    ]);
    frame.render_widget(Paragraph::new(assignee_line), sections[1]);

    let footer = if draft.is_submitting {
        Line::from(Span::styled(
            "Submitting issue...",
            Style::default().fg(Color::Yellow),
        ))
    } else if let Some(error) = &draft.validation_error {
        Line::from(Span::styled(error.clone(), Style::default().fg(Color::Red)))
    } else if let Some(info) = &draft.info_message {
        Line::from(Span::styled(
            info.clone(),
            Style::default().fg(Color::Green),
        ))
    } else if draft.editing {
        Line::from(Span::styled(
            "Ctrl+Enter to validate. Ctrl+N to stop editing.",
            Style::default().fg(Color::DarkGray),
        ))
    } else {
        Line::from(Span::styled(
            "Ctrl+N to edit prompt. Ctrl+Enter to validate.",
            Style::default().fg(Color::DarkGray),
        ))
    };
    frame.render_widget(Paragraph::new(footer), sections[2]);
}

struct PromptRender {
    lines: Vec<Line<'static>>,
    cursor: Option<(u16, u16)>,
}

fn build_prompt_render(draft: &IssueDraft, area: ratatui::layout::Rect) -> PromptRender {
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        "Prompt",
        Style::default().add_modifier(Modifier::BOLD),
    )));

    if draft.prompt.trim().is_empty() {
        let hint = if draft.editing {
            "Type to describe the work for a new issue."
        } else {
            "Press Ctrl+N to start editing."
        };

        if draft.editing {
            lines.push(Line::from(Span::raw("")));
        }

        lines.push(Line::from(Span::styled(
            "Describe the work to create a new issue.",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(Span::styled(
            hint,
            Style::default().fg(Color::DarkGray),
        )));

        let cursor = if draft.editing {
            Some((area.x, area.y.saturating_add(1)))
        } else {
            None
        };

        return PromptRender { lines, cursor };
    }

    let prompt_lines: Vec<&str> = draft.prompt.split('\n').collect();
    lines.extend(
        prompt_lines
            .iter()
            .map(|line| Line::from(Span::raw((*line).to_string()))),
    );

    let cursor = if draft.editing {
        prompt_cursor_position(&prompt_lines, area)
    } else {
        None
    };

    PromptRender { lines, cursor }
}

fn prompt_cursor_position(
    prompt_lines: &[&str],
    area: ratatui::layout::Rect,
) -> Option<(u16, u16)> {
    let width = area.width as usize;
    let height = area.height as usize;
    if width == 0 || height == 0 {
        return None;
    }

    let mut visual_lines = Vec::new();
    for line in prompt_lines {
        if line.is_empty() {
            visual_lines.push(String::new());
            continue;
        }
        let wrapped = textwrap::wrap(line, width);
        if wrapped.is_empty() {
            visual_lines.push(String::new());
        } else {
            for chunk in wrapped {
                visual_lines.push(chunk.into_owned());
            }
        }
    }

    if visual_lines.is_empty() {
        visual_lines.push(String::new());
    }

    let last_line = visual_lines.last().unwrap();
    let max_x = area.x.saturating_add(area.width.saturating_sub(1));
    let max_y = area.y.saturating_add(area.height.saturating_sub(1));

    let cursor_x = area
        .x
        .saturating_add(last_line.len().min(width.saturating_sub(1)) as u16)
        .min(max_x);
    let cursor_y = area
        .y
        .saturating_add(1)
        .saturating_add(visual_lines.len().saturating_sub(1) as u16)
        .min(max_y);

    Some((cursor_x, cursor_y))
}

fn render_issue_list(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    issue_lines: &IssueLines,
    title: &str,
    empty_message: &str,
) {
    let items: Vec<ListItem> = if issue_lines.rows.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            empty_message,
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        issue_lines
            .rows
            .iter()
            .map(|line| {
                let mut spans = Vec::new();
                spans.push(Span::raw(issue_prefix(line.depth)));
                spans.push(Span::raw(" "));
                let (issue_status_label, issue_status_style) =
                    issue_status_display(line.status, &line.readiness);
                spans.push(Span::styled(
                    format!("[{issue_status_label}]"),
                    issue_status_style,
                ));

                if let Some(task) = &line.task {
                    if let Some(runtime) = &task.runtime {
                        spans.push(Span::raw(" "));
                        spans.push(Span::styled(
                            format!("[{runtime}]"),
                            status_style(task.status),
                        ));
                    }
                }

                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    line.id.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ));
                if let Some(assignee) = &line.assignee {
                    spans.push(Span::raw(" "));
                    spans.push(Span::styled(
                        format!("@{assignee}"),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                spans.push(Span::raw(" — "));
                spans.push(Span::raw(truncate_message(
                    &line.summary,
                    MAX_MESSAGE_WIDTH,
                )));
                if let Some(progress) = &line.progress {
                    spans.push(Span::raw(" "));
                    spans.push(Span::styled(
                        truncate_message(progress, MAX_MESSAGE_WIDTH),
                        Style::default().fg(Color::DarkGray),
                    ));
                }

                ListItem::new(Line::from(spans))
            })
            .collect()
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::White));
    frame.render_widget(List::new(items).block(block), area);
}

fn issue_prefix(depth: usize) -> String {
    if depth == 0 {
        "|".to_string()
    } else {
        format!("│{}", "  ".repeat(depth))
    }
}

async fn refresh_jobs(
    client: &dyn MetisClientInterface,
    state: &mut DashboardState,
) -> Result<bool> {
    let response = client.list_jobs(&SearchJobsQuery::default()).await?;
    let now = Utc::now();

    let previous_jobs = state.jobs.clone();
    let mut jobs = Vec::new();
    for summary in response.jobs {
        let issue_id = match cached_issue_id(&previous_jobs, &summary.id) {
            Some(id) => id,
            None => fetch_issue_id(client, &summary.id).await?,
        };
        let display = summarize_job(summary, now);
        jobs.push(JobDetails { display, issue_id });
    }

    let jobs_changed = jobs != state.jobs;
    if jobs_changed {
        state.jobs = jobs;
    }

    let derived_changed = update_views(state);
    Ok(jobs_changed || derived_changed)
}

async fn refresh_records(
    client: &dyn MetisClientInterface,
    state: &mut DashboardState,
) -> Result<bool> {
    let issues = fetch_issues(client).await?;

    let changed = issues != state.issues;
    if changed {
        state.issues = issues;
    }

    let derived_changed = update_views(state);
    Ok(changed || derived_changed)
}

fn cached_issue_id(previous_jobs: &[JobDetails], job_id: &TaskId) -> Option<Option<IssueId>> {
    previous_jobs
        .iter()
        .find(|job| job.display.id == *job_id)
        .map(|job| job.issue_id.clone())
}

async fn fetch_issue_id(
    client: &dyn MetisClientInterface,
    job_id: &TaskId,
) -> Result<Option<IssueId>> {
    let context = client
        .get_job_context(job_id)
        .await
        .with_context(|| format!("failed to fetch job context for '{job_id}'"))?;

    let issue_id = context
        .variables
        .get(ISSUE_ID_VAR)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<IssueId>().ok());

    Ok(issue_id)
}

async fn fetch_issues(client: &dyn MetisClientInterface) -> Result<Vec<IssueRecord>> {
    let response = client
        .list_issues(&SearchIssuesQuery::default())
        .await
        .context("failed to fetch issues")?;

    let issues = response
        .issues
        .into_iter()
        .filter_map(issue_to_record)
        .collect();

    Ok(issues)
}

fn issue_to_record(record: ApiIssueRecord) -> Option<IssueRecord> {
    let issue = record.issue;
    Some(IssueRecord {
        id: record.id,
        description: issue.description,
        progress: issue.progress,
        status: issue.status,
        assignee: issue.assignee,
        dependencies: issue.dependencies,
    })
}

fn update_views(state: &mut DashboardState) -> bool {
    let previous_issue_lines = state.issue_lines.clone();
    let previous_assigned_issue_lines = state.assigned_issue_lines.clone();
    let previous_assignee_options = state.issue_draft.assignees.clone();
    let previous_assignee_index = state.issue_draft.assignee_index;

    let issue_lines = build_issue_lines(&state.issues, &state.jobs);
    let assigned_issue_lines =
        build_assigned_issue_lines(state.username.as_deref(), &state.issues, &state.jobs);
    update_assignee_options(state);

    state.issue_lines = issue_lines;
    state.assigned_issue_lines = assigned_issue_lines;

    previous_issue_lines != state.issue_lines
        || previous_assigned_issue_lines != state.assigned_issue_lines
        || previous_assignee_options != state.issue_draft.assignees
        || previous_assignee_index != state.issue_draft.assignee_index
}

fn update_assignee_options(state: &mut DashboardState) {
    let options = build_assignee_options(&state.issues);
    if options != state.issue_draft.assignees {
        state.issue_draft.assignees = options;
    }

    let fallback = "pm";
    let preferred = state.issue_draft.selected_assignee().unwrap_or(fallback);
    let next_index = state
        .issue_draft
        .assignees
        .iter()
        .position(|assignee| assignee == preferred)
        .or_else(|| {
            state
                .issue_draft
                .assignees
                .iter()
                .position(|assignee| assignee == fallback)
        })
        .unwrap_or(0);
    state.issue_draft.assignee_index = next_index;
}

fn build_assignee_options(issues: &[IssueRecord]) -> Vec<String> {
    let mut options = BTreeSet::new();
    options.insert("pm".to_string());

    for issue in issues {
        if let Some(assignee) = &issue.assignee {
            let trimmed = assignee.trim();
            if !trimmed.is_empty() {
                options.insert(trimmed.to_string());
            }
        }
    }

    options.into_iter().collect()
}

fn build_assigned_issue_lines(
    username: Option<&str>,
    issues: &[IssueRecord],
    jobs: &[JobDetails],
) -> IssueLines {
    let Some(username) = username.map(str::trim).filter(|value| !value.is_empty()) else {
        return IssueLines::default();
    };

    let assigned: Vec<IssueRecord> = issues
        .iter()
        .filter(|issue| {
            matches!(issue.status, IssueStatus::Open | IssueStatus::InProgress)
                && issue.assignee.as_deref() == Some(username)
        })
        .cloned()
        .collect();

    build_issue_lines(&assigned, jobs)
}

fn build_issue_lines(issues: &[IssueRecord], jobs: &[JobDetails]) -> IssueLines {
    let mut tasks_by_issue: HashMap<IssueId, Vec<JobDisplay>> = HashMap::new();

    for job in jobs {
        if let Some(issue_id) = &job.issue_id {
            tasks_by_issue
                .entry(issue_id.clone())
                .or_default()
                .push(job.display.clone());
        }
    }

    let mut nodes: HashMap<IssueId, IssueNode> = issues
        .iter()
        .map(|issue| {
            let task = tasks_by_issue
                .get(&issue.id)
                .and_then(|tasks| best_task_indicator(tasks));
            (
                issue.id.clone(),
                IssueNode {
                    record: issue.clone(),
                    parent: None,
                    children: Vec::new(),
                    task,
                },
            )
        })
        .collect();

    let ids: Vec<IssueId> = nodes.keys().cloned().collect();
    for id in &ids {
        let parent = nodes.get(id).and_then(|node| {
            node.record
                .dependencies
                .iter()
                .find(|dep| dep.dependency_type == IssueDependencyType::ChildOf)
                .map(|dep| dep.issue_id.clone())
        });

        if let Some(parent_id) = parent {
            if let Some(parent_node) = nodes.get_mut(&parent_id) {
                parent_node.children.push(id.clone());
                if let Some(node) = nodes.get_mut(id) {
                    node.parent = Some(parent_id);
                }
            }
        }
    }

    let mut roots: Vec<IssueId> = nodes
        .iter()
        .filter(|(_, node)| node.parent.is_none())
        .map(|(id, _)| id.clone())
        .collect();
    roots.sort_by(|a, b| compare_issue_nodes(&nodes, a, b));

    let mut rows = Vec::new();
    let mut visited: HashSet<IssueId> = HashSet::new();
    for root in roots {
        append_issue(&root, 0, &mut rows, &mut visited, &nodes);
    }

    IssueLines { rows }
}

fn append_issue(
    id: &IssueId,
    depth: usize,
    rows: &mut Vec<IssueLine>,
    visited: &mut HashSet<IssueId>,
    nodes: &HashMap<IssueId, IssueNode>,
) {
    if !visited.insert(id.clone()) {
        return;
    }

    let Some(node) = nodes.get(id) else {
        return;
    };

    let readiness = issue_readiness(node, nodes);
    let issue_summary = issue_summary(&node.record.description, &node.record.progress);

    rows.push(IssueLine {
        id: node.record.id.to_string(),
        summary: issue_summary.summary,
        progress: issue_summary.progress,
        status: node.record.status,
        readiness,
        assignee: node.record.assignee.clone(),
        task: node.task.clone(),
        depth,
    });

    let mut children = node.children.clone();
    children.sort_by(|a, b| compare_issue_nodes(nodes, a, b));
    for child in children {
        append_issue(&child, depth + 1, rows, visited, nodes);
    }
}

fn issue_summary(description: &str, progress: &str) -> IssueSummary {
    let description = description.trim();
    let progress = progress.trim();

    let summary = if description.is_empty() {
        "-".to_string()
    } else {
        description.to_string()
    };

    let progress = if progress.is_empty() {
        None
    } else {
        Some(progress.to_string())
    };

    IssueSummary { summary, progress }
}

fn active_blockers(node: &IssueNode, nodes: &HashMap<IssueId, IssueNode>) -> Vec<String> {
    node.record
        .dependencies
        .iter()
        .filter(|dep| dep.dependency_type == IssueDependencyType::BlockedOn)
        .filter(|dep| {
            nodes
                .get(&dep.issue_id)
                .map(|blocker| blocker.record.status != IssueStatus::Closed)
                .unwrap_or(true)
        })
        .map(|dep| dep.issue_id.to_string())
        .collect()
}

fn has_open_children(node: &IssueNode, nodes: &HashMap<IssueId, IssueNode>) -> bool {
    node.children.iter().any(|child_id| {
        nodes
            .get(child_id)
            .map(|child| child.record.status != IssueStatus::Closed)
            .unwrap_or(true)
    })
}

fn issue_readiness(node: &IssueNode, nodes: &HashMap<IssueId, IssueNode>) -> IssueReadiness {
    if node.record.status == IssueStatus::Dropped {
        return IssueReadiness::Dropped;
    }

    let blockers = active_blockers(node, nodes);
    if !blockers.is_empty() {
        return IssueReadiness::Blocked(blockers);
    }

    if node.record.status == IssueStatus::InProgress && has_open_children(node, nodes) {
        return IssueReadiness::Waiting;
    }

    IssueReadiness::Ready
}

fn compare_issue_nodes(nodes: &HashMap<IssueId, IssueNode>, a: &IssueId, b: &IssueId) -> Ordering {
    let Some(left) = nodes.get(a) else {
        return Ordering::Less;
    };
    let Some(right) = nodes.get(b) else {
        return Ordering::Greater;
    };

    issue_status_order(left.record.status)
        .cmp(&issue_status_order(right.record.status))
        .then_with(|| left.record.id.cmp(&right.record.id))
}

fn best_task_indicator(tasks: &[JobDisplay]) -> Option<TaskIndicator> {
    tasks
        .iter()
        .min_by(|a, b| {
            task_status_order(a.status)
                .cmp(&task_status_order(b.status))
                .then_with(|| compare_recent(a.last_change, b.last_change))
        })
        .map(|job| TaskIndicator {
            status: job.status,
            runtime: match job.status {
                Status::Running | Status::Complete | Status::Failed => job.runtime.clone(),
                _ => None,
            },
        })
}

fn task_status_order(status: Status) -> usize {
    match status {
        Status::Running => 0,
        Status::Pending => 1,
        Status::Failed => 2,
        Status::Complete => 3,
    }
}

fn issue_status_order(status: IssueStatus) -> usize {
    match status {
        IssueStatus::InProgress => 0,
        IssueStatus::Open => 1,
        IssueStatus::Dropped => 2,
        IssueStatus::Closed => 3,
    }
}

fn summarize_job(job: JobRecord, now: DateTime<Utc>) -> JobDisplay {
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

fn note_or_error(job: &JobRecord) -> String {
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

fn status_style(status: Status) -> Style {
    match status {
        Status::Complete => Style::default().fg(Color::Green),
        Status::Running => Style::default().fg(Color::Yellow),
        Status::Failed => Style::default().fg(Color::Red),
        Status::Pending => Style::default().fg(Color::Blue),
    }
}

fn issue_status_style(status: IssueStatus) -> Style {
    match status {
        IssueStatus::Open => Style::default().fg(Color::Blue),
        IssueStatus::InProgress => Style::default().fg(Color::Yellow),
        IssueStatus::Closed => Style::default().fg(Color::Green),
        IssueStatus::Dropped => Style::default().fg(Color::Rgb(139, 0, 0)),
    }
}

fn issue_status_display(status: IssueStatus, readiness: &IssueReadiness) -> (String, Style) {
    match (status, readiness) {
        (IssueStatus::Dropped, _) => (
            "dropped".to_string(),
            issue_status_style(IssueStatus::Dropped),
        ),
        (IssueStatus::Open, IssueReadiness::Blocked(blockers)) => (
            format!("blocked: {}", blockers.join(", ")),
            Style::default().fg(Color::Magenta),
        ),
        (IssueStatus::Closed, _) => (
            "closed".to_string(),
            issue_status_style(IssueStatus::Closed),
        ),
        (IssueStatus::InProgress, _) => (
            "in-progress".to_string(),
            issue_status_style(IssueStatus::InProgress),
        ),
        (IssueStatus::Open, _) => ("open".to_string(), issue_status_style(IssueStatus::Open)),
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
    use crate::client::MockMetisClient;
    use crate::test_utils::ids::{issue_id, task_id};
    use chrono::Duration as ChronoDuration;
    use metis_common::issues::UpsertIssueResponse;
    use metis_common::jobs::{BundleSpec, Task};
    use metis_common::task_status::Event;
    use std::collections::HashMap;

    fn job_with_status(id: &str, status: Status, offset_seconds: i64) -> JobRecord {
        let now = Utc::now() - ChronoDuration::seconds(offset_seconds);
        let mut log = TaskStatusLog::new(Status::Pending, now);
        match status {
            Status::Running => log.events.push(Event::Started { at: now }),
            Status::Complete => {
                log.events.push(Event::Started { at: now });
                log.events.push(Event::Completed {
                    at: now,
                    last_message: None,
                });
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
            Status::Pending => {}
        }

        JobRecord {
            id: task_id(id),
            task: Task {
                prompt: "0".into(),
                context: BundleSpec::None,
                spawned_from: None,
                image: None,
                env_vars: HashMap::new(),
            },
            notes: None,
            status_log: log,
        }
    }

    fn issue(id: &str, status: IssueStatus, dependencies: Vec<IssueDependency>) -> IssueRecord {
        IssueRecord {
            id: issue_id(id),
            description: id.to_string(),
            progress: String::new(),
            status,
            assignee: None,
            dependencies,
        }
    }

    fn issue_with_assignee(id: &str, status: IssueStatus, assignee: Option<&str>) -> IssueRecord {
        IssueRecord {
            id: issue_id(id),
            description: id.to_string(),
            progress: String::new(),
            status,
            assignee: assignee.map(str::to_string),
            dependencies: Vec::new(),
        }
    }

    fn job_details_with_issue(
        id: &str,
        status: Status,
        linked_issue: Option<&str>,
        runtime: Option<&str>,
    ) -> JobDetails {
        JobDetails {
            display: JobDisplay {
                id: task_id(id),
                status,
                runtime: runtime.map(|value| value.to_string()),
                note: "-".to_string(),
                last_change: Some(Utc::now()),
            },
            issue_id: linked_issue.map(issue_id),
        }
    }

    #[test]
    fn note_or_error_prefers_error_reason() {
        let mut job = job_with_status("t-job-failed", Status::Failed, 0);
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

    #[test]
    fn issue_lines_sorted_by_status_and_nested() {
        let issues = vec![
            issue("i-1", IssueStatus::Open, vec![]),
            issue(
                "i-3",
                IssueStatus::Closed,
                vec![IssueDependency {
                    dependency_type: IssueDependencyType::ChildOf,
                    issue_id: issue_id("i-1"),
                }],
            ),
            issue("i-2", IssueStatus::InProgress, vec![]),
        ];

        let lines = build_issue_lines(&issues, &[]);

        assert_eq!(lines.rows.len(), 3);
        assert_eq!(lines.rows[0].id, issue_id("i-2").to_string());
        assert_eq!(lines.rows[1].id, issue_id("i-1").to_string());
        assert_eq!(lines.rows[2].id, issue_id("i-3").to_string());
        assert_eq!(lines.rows[2].depth, 1);
    }

    #[test]
    fn blocked_on_excludes_closed_dependencies() {
        let issues = vec![
            issue(
                "i-1",
                IssueStatus::Open,
                vec![
                    IssueDependency {
                        dependency_type: IssueDependencyType::BlockedOn,
                        issue_id: issue_id("i-closed"),
                    },
                    IssueDependency {
                        dependency_type: IssueDependencyType::BlockedOn,
                        issue_id: issue_id("i-open"),
                    },
                ],
            ),
            issue("i-closed", IssueStatus::Closed, vec![]),
            issue("i-open", IssueStatus::Open, vec![]),
        ];

        let lines = build_issue_lines(&issues, &[]);

        let blocked_line = lines
            .rows
            .iter()
            .find(|line| line.id == issue_id("i-1").to_string())
            .expect("issue line missing");
        match &blocked_line.readiness {
            IssueReadiness::Blocked(blockers) => {
                assert_eq!(blockers, &[issue_id("i-open").to_string()])
            }
            other => panic!("unexpected readiness: {other:?}"),
        }
    }

    #[test]
    fn dropped_issues_render_as_dropped() {
        let issues = vec![issue("i-drop", IssueStatus::Dropped, vec![])];
        let lines = build_issue_lines(&issues, &[]);

        let line = lines.rows.first().expect("missing issue line");
        assert_eq!(line.readiness, IssueReadiness::Dropped);

        let (label, style) = issue_status_display(line.status, &line.readiness);
        assert_eq!(label, "dropped");
        assert_eq!(style, issue_status_style(IssueStatus::Dropped));
    }

    #[test]
    fn issue_lines_include_task_indicator() {
        let issues = vec![issue("i-1", IssueStatus::Open, vec![])];
        let jobs = vec![job_details_with_issue(
            "t-job-1",
            Status::Running,
            Some("i-1"),
            Some("3s"),
        )];

        let lines = build_issue_lines(&issues, &jobs);

        let line = lines.rows.first().expect("issue line missing");
        let task = line.task.as_ref().expect("task indicator missing");
        assert_eq!(task.status, Status::Running);
        assert_eq!(task.runtime.as_deref(), Some("3s"));
    }

    #[test]
    fn failed_tasks_keep_runtime_in_indicator() {
        let jobs = vec![
            job_details_with_issue("t-failed", Status::Failed, Some("i-1"), Some("8s")).display,
        ];

        let indicator = best_task_indicator(&jobs).expect("missing task indicator");

        assert_eq!(indicator.status, Status::Failed);
        assert_eq!(indicator.runtime.as_deref(), Some("8s"));
    }

    #[test]
    fn issue_lines_include_progress() {
        let issues = vec![IssueRecord {
            id: issue_id("i-progress"),
            description: "investigate logs".into(),
            progress: "drafting tests".into(),
            status: IssueStatus::Open,
            assignee: None,
            dependencies: Vec::new(),
        }];

        let lines = build_issue_lines(&issues, &[]);

        let line = lines.rows.first().expect("issue line missing");
        assert_eq!(line.summary, "investigate logs");
        assert_eq!(line.progress.as_deref(), Some("drafting tests"));
    }

    #[test]
    fn in_progress_issues_show_waiting_when_children_open() {
        let issues = vec![
            issue(
                "i-parent",
                IssueStatus::InProgress,
                vec![IssueDependency {
                    dependency_type: IssueDependencyType::ChildOf,
                    issue_id: issue_id("i-root"),
                }],
            ),
            issue(
                "i-root",
                IssueStatus::InProgress,
                vec![IssueDependency {
                    dependency_type: IssueDependencyType::ChildOf,
                    issue_id: issue_id("i-grand"),
                }],
            ),
            issue("i-grand", IssueStatus::Open, vec![]),
        ];

        let lines = build_issue_lines(&issues, &[]);

        let line = lines
            .rows
            .iter()
            .find(|line| line.id == issue_id("i-root").to_string())
            .expect("issue line missing");
        assert_eq!(line.status, IssueStatus::InProgress);
        assert!(matches!(line.readiness, IssueReadiness::Waiting));

        let (label, style) = issue_status_display(line.status, &line.readiness);
        assert_eq!(label, "in-progress");
        assert_eq!(style, issue_status_style(IssueStatus::InProgress));
    }

    #[test]
    fn assigned_issue_lines_filter_assignee_and_status() {
        let issues = vec![
            issue_with_assignee("i-open", IssueStatus::Open, Some("alice")),
            issue_with_assignee("i-in-progress", IssueStatus::InProgress, Some("alice")),
            issue_with_assignee("i-closed", IssueStatus::Closed, Some("alice")),
            issue_with_assignee("i-other", IssueStatus::Open, Some("bob")),
        ];

        let lines = build_assigned_issue_lines(Some("alice"), &issues, &[]);

        assert_eq!(lines.rows.len(), 2);
        assert!(lines
            .rows
            .iter()
            .any(|line| line.id == issue_id("i-open").to_string()));
        assert!(lines
            .rows
            .iter()
            .any(|line| line.id == issue_id("i-in-progress").to_string()));
    }

    #[test]
    fn build_assignee_options_includes_pm_and_unique_sorted() {
        let issues = vec![
            issue_with_assignee("i-1", IssueStatus::Open, Some("alice")),
            issue_with_assignee("i-2", IssueStatus::Open, Some("bob")),
            issue_with_assignee("i-3", IssueStatus::Open, Some("alice")),
        ];

        let options = build_assignee_options(&issues);

        assert!(options.contains(&"pm".to_string()));
        assert!(options.contains(&"alice".to_string()));
        assert!(options.contains(&"bob".to_string()));
        assert_eq!(options.len(), 3);
    }

    #[test]
    fn attempt_issue_submit_requires_prompt() {
        let mut state = DashboardState::default();
        state.issue_draft.prompt = "   ".to_string();

        let submission = attempt_issue_submit(&mut state);

        assert!(submission.is_none());
        assert_eq!(
            state.issue_draft.validation_error.as_deref(),
            Some("Prompt cannot be empty.")
        );
        assert!(!state.issue_draft.is_submitting);
    }

    #[test]
    fn attempt_issue_submit_sets_loading_state() {
        let mut state = DashboardState::default();
        state.issue_draft.prompt = "Ship dashboard".to_string();
        state.issue_draft.assignees = vec!["pm".to_string()];

        let submission = attempt_issue_submit(&mut state).expect("submission missing");

        assert_eq!(submission.prompt, "Ship dashboard");
        assert_eq!(submission.assignee, "pm");
        assert!(state.issue_draft.is_submitting);
    }

    #[tokio::test]
    async fn submit_issue_sends_task_request() {
        let client = MockMetisClient::default();
        client.push_upsert_issue_response(UpsertIssueResponse {
            issue_id: issue_id("i-new"),
        });

        let submission = IssueSubmission {
            prompt: "Draft release notes".to_string(),
            assignee: "alice".to_string(),
        };

        let created = submit_issue(&client, &submission)
            .await
            .expect("submission failed");

        assert_eq!(created, issue_id("i-new"));

        let requests = client.recorded_issue_upserts();
        assert_eq!(requests.len(), 1);
        let (_, request) = &requests[0];
        assert_eq!(request.issue.issue_type, IssueType::Task);
        assert_eq!(request.issue.status, IssueStatus::Open);
        assert_eq!(request.issue.description, "Draft release notes");
        assert_eq!(request.issue.assignee.as_deref(), Some("alice"));
        assert!(request.issue.dependencies.is_empty());
    }
}
