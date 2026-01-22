use std::{
    cmp::Ordering,
    collections::{BTreeSet, HashMap, HashSet},
    time::Duration,
};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use crossterm::event::{
    Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};
use futures::StreamExt;
use metis_common::{
    issues::{
        Issue, IssueDependency, IssueDependencyType, IssueRecord as ApiIssueRecord, IssueStatus,
        IssueType, SearchIssuesQuery, UpsertIssueRequest,
    },
    jobs::{JobRecord, SearchJobsQuery},
    task_status::{Status, TaskError, TaskStatusLog},
    users::{User, Username},
    IssueId, TaskId,
};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    DefaultTerminal, Frame,
};
use tui_textarea::TextArea;

use crate::{auth, client::MetisClientInterface, command::jobs};

pub mod panel;

use panel::{wrapped_content_len, Panel, PanelEvent, PanelState};

const JOB_REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const RECORD_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const MAX_MESSAGE_WIDTH: usize = 90;
const ISSUE_ID_VAR: &str = "METIS_ISSUE_ID";
const USER_ISSUES_PANEL_CONTENT_HEIGHT: u16 = 5;
const USER_ISSUES_PANEL_HEIGHT: u16 = USER_ISSUES_PANEL_CONTENT_HEIGHT + 2;
const ISSUE_CREATOR_PANEL_INNER_HEIGHT: u16 = 10;
const ISSUE_CREATOR_PANEL_HEIGHT: u16 = ISSUE_CREATOR_PANEL_INNER_HEIGHT + 2;
#[derive(Copy, Clone, PartialEq, Default, Debug)]
enum PanelFocus {
    #[default]
    NewIssue,
    UserOwned,
    Running,
    Completed,
}

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

#[derive(Default, Clone, PartialEq)]
struct CompletedIssueLines {
    roots: Vec<IssueLine>,
    descendants: HashMap<IssueId, Vec<IssueLine>>,
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

#[derive(Clone)]
struct IssueDraft {
    prompt: TextArea<'static>,
    assignees: Vec<String>,
    assignee_index: usize,
    validation_error: Option<String>,
    info_message: Option<String>,
    is_submitting: bool,
}

impl Default for IssueDraft {
    fn default() -> Self {
        let mut draft = Self {
            prompt: TextArea::default(),
            assignees: vec!["pm".to_string()],
            assignee_index: 0,
            validation_error: None,
            info_message: None,
            is_submitting: false,
        };
        draft.configure_prompt(false);
        draft
    }
}

impl IssueDraft {
    fn prompt_text(&self) -> String {
        self.prompt.lines().join("\n")
    }

    #[cfg(test)]
    fn set_prompt(&mut self, prompt: &str, focused: bool) {
        self.prompt = TextArea::from(prompt.lines());
        self.configure_prompt(focused);
    }

    fn clear_prompt(&mut self, focused: bool) {
        self.prompt = TextArea::default();
        self.configure_prompt(focused);
    }

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

    fn configure_prompt(&mut self, focused: bool) {
        let placeholder = "Describe the work to create a new issue.";
        self.prompt.set_placeholder_text(placeholder);
        self.prompt
            .set_placeholder_style(Style::default().fg(Color::DarkGray));
        self.prompt.set_style(Style::default());
        if focused {
            self.prompt
                .set_cursor_line_style(Style::default().add_modifier(Modifier::UNDERLINED));
            self.prompt
                .set_cursor_style(Style::default().add_modifier(Modifier::REVERSED));
        } else {
            self.prompt.set_cursor_line_style(Style::default());
            self.prompt.set_cursor_style(Style::default());
        }
    }
}

impl PartialEq for IssueDraft {
    fn eq(&self, other: &Self) -> bool {
        self.prompt_text() == other.prompt_text()
            && self.assignees == other.assignees
            && self.assignee_index == other.assignee_index
            && self.validation_error == other.validation_error
            && self.info_message == other.info_message
            && self.is_submitting == other.is_submitting
    }
}

#[derive(Default, Clone, PartialEq)]
struct ListScrollState {
    offset: usize,
    scrollbar_state: ScrollbarState,
}

#[derive(Clone)]
struct DashboardState {
    jobs: Vec<JobDetails>,
    issues: Vec<IssueRecord>,
    issue_lines: IssueLines,
    user_unowned_issue_lines: IssueLines,
    completed_issue_lines: CompletedIssueLines,
    running_issue_panel: PanelState,
    user_unowned_issue_panel: PanelState,
    completed_issue_panel: PanelState,
    issue_creator_panel: PanelState,
    issue_draft_scroll: ListScrollState,
    jobs_error: Option<String>,
    records_error: Option<String>,
    username: String,
    issue_draft: IssueDraft,
    selected_panel: PanelFocus,
    last_frame_size: Option<Rect>,
}

impl Default for DashboardState {
    fn default() -> Self {
        let mut issue_creator_panel = PanelState::new();
        issue_creator_panel.set_scroll_keys_enabled(false);
        issue_creator_panel.register_keybinding(KeyCode::Char('a'), KeyModifiers::ALT, "Assignee");
        issue_creator_panel.register_keybinding(KeyCode::Enter, KeyModifiers::ALT, "Submit");
        issue_creator_panel.register_keybinding(KeyCode::Tab, KeyModifiers::NONE, "Next panel");
        issue_creator_panel.register_keybinding(KeyCode::BackTab, KeyModifiers::NONE, "Prev panel");

        let mut running_issue_panel = PanelState::new();
        configure_status_panel_keybindings(&mut running_issue_panel);
        let mut user_unowned_issue_panel = PanelState::new();
        configure_status_panel_keybindings(&mut user_unowned_issue_panel);
        let mut completed_issue_panel = PanelState::new();
        configure_status_panel_keybindings(&mut completed_issue_panel);

        let mut state = Self {
            jobs: Vec::new(),
            issues: Vec::new(),
            issue_lines: IssueLines::default(),
            user_unowned_issue_lines: IssueLines::default(),
            completed_issue_lines: CompletedIssueLines::default(),
            running_issue_panel,
            user_unowned_issue_panel,
            completed_issue_panel,
            issue_creator_panel,
            issue_draft_scroll: ListScrollState::default(),
            jobs_error: None,
            records_error: None,
            username: String::new(),
            issue_draft: IssueDraft::default(),
            selected_panel: PanelFocus::default(),
            last_frame_size: None,
        };
        update_panel_focus(&mut state);
        state
    }
}

fn configure_status_panel_keybindings(panel: &mut PanelState) {
    panel.register_keybinding(KeyCode::Tab, KeyModifiers::NONE, "Next panel");
    panel.register_keybinding(KeyCode::BackTab, KeyModifiers::NONE, "Prev panel");
}

struct IssueSubmission {
    prompt: String,
    assignee: String,
}

struct EventOutcome {
    should_quit: bool,
    submission: Option<IssueSubmission>,
}

pub async fn run(client: &dyn MetisClientInterface) -> Result<()> {
    let username = auth::resolve_auth_user(client).await?.to_string();
    let mut terminal = ratatui::init();
    let result = run_dashboard_loop(client, &mut terminal, username).await;
    ratatui::restore();
    result
}

async fn run_dashboard_loop(
    client: &dyn MetisClientInterface,
    terminal: &mut DefaultTerminal,
    username: String,
) -> Result<()> {
    let mut state = DashboardState {
        username,
        ..DashboardState::default()
    };
    update_panel_focus(&mut state);
    let mut needs_draw = true;

    if let Err(err) = refresh_jobs(client, &mut state).await {
        state.jobs_error = Some(format!("Failed to load jobs: {err}"));
    }

    if let Err(err) = refresh_records(client, &mut state).await {
        state.records_error = Some(format!("Failed to load issues: {err}"));
    }

    let mut events = EventStream::new();
    let mut jobs_tick = tokio::time::interval(JOB_REFRESH_INTERVAL);
    let mut records_tick = tokio::time::interval(RECORD_REFRESH_INTERVAL);

    loop {
        if needs_draw {
            state.last_frame_size = Some(terminal.size()?.into());
            clamp_issue_scrolls(&mut state);
            terminal.draw(|f| render(f, &mut state))?;
            needs_draw = false;
        }

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
                            terminal.draw(|f| render(f, &mut state))?;
                            let submission_result =
                                submit_issue(client, &submission, &state.username)
                            .await;
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
    }

    Ok(())
}

fn handle_event(event: Event, state: &mut DashboardState) -> EventOutcome {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                return EventOutcome {
                    should_quit: true,
                    submission: None,
                };
            }

            if is_panel_focus_key(key) {
                handle_panel_focus_key(key, state);
                return EventOutcome {
                    should_quit: false,
                    submission: None,
                };
            }

            let submission = match state.selected_panel {
                PanelFocus::NewIssue => handle_issue_draft_key(key, state),
                PanelFocus::UserOwned | PanelFocus::Running | PanelFocus::Completed => {
                    handle_status_panel_key(key, state);
                    None
                }
            };
            EventOutcome {
                should_quit: false,
                submission,
            }
        }
        Event::Paste(text) => {
            if state.issue_draft.is_submitting {
                return EventOutcome {
                    should_quit: false,
                    submission: None,
                };
            }

            if state.selected_panel == PanelFocus::NewIssue
                && state.issue_draft.prompt.insert_str(text)
            {
                state.issue_draft.note_edit();
                state.selected_panel = PanelFocus::NewIssue;
            }

            EventOutcome {
                should_quit: false,
                submission: None,
            }
        }
        Event::Resize(width, height) => {
            state.last_frame_size = Some(Rect::new(0, 0, width, height));
            clamp_issue_scrolls(state);
            EventOutcome {
                should_quit: false,
                submission: None,
            }
        }
        Event::Mouse(mouse) => {
            handle_mouse_scroll(mouse, state);
            handle_mouse_click(mouse, state);
            EventOutcome {
                should_quit: false,
                submission: None,
            }
        }
        _ => EventOutcome {
            should_quit: false,
            submission: None,
        },
    }
}

fn has_alt_modifier(modifiers: KeyModifiers) -> bool {
    modifiers.contains(KeyModifiers::ALT)
        && !modifiers.contains(KeyModifiers::CONTROL)
        && !modifiers.contains(KeyModifiers::META)
}

fn is_alt_char_key(key: KeyEvent, target: char) -> bool {
    matches!(key.code, KeyCode::Char(ch) if ch.eq_ignore_ascii_case(&target))
        && has_alt_modifier(key.modifiers)
}

fn is_panel_focus_key(key: KeyEvent) -> bool {
    match key.code {
        KeyCode::BackTab => true,
        KeyCode::Tab => key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT,
        _ => false,
    }
}

fn is_issue_submit_key(key: KeyEvent) -> bool {
    key.code == KeyCode::Enter && has_alt_modifier(key.modifiers)
}

fn handle_panel_focus_key(key: KeyEvent, state: &mut DashboardState) {
    state.selected_panel =
        if key.code == KeyCode::BackTab || key.modifiers.contains(KeyModifiers::SHIFT) {
            prev_panel_focus(state.selected_panel)
        } else {
            next_panel_focus(state.selected_panel)
        };
    update_panel_focus(state);
}

fn next_panel_focus(current: PanelFocus) -> PanelFocus {
    match current {
        PanelFocus::NewIssue => PanelFocus::UserOwned,
        PanelFocus::UserOwned => PanelFocus::Running,
        PanelFocus::Running => PanelFocus::Completed,
        PanelFocus::Completed => PanelFocus::NewIssue,
    }
}

fn prev_panel_focus(current: PanelFocus) -> PanelFocus {
    match current {
        PanelFocus::NewIssue => PanelFocus::Completed,
        PanelFocus::UserOwned => PanelFocus::NewIssue,
        PanelFocus::Running => PanelFocus::UserOwned,
        PanelFocus::Completed => PanelFocus::Running,
    }
}

fn update_panel_focus(state: &mut DashboardState) {
    state
        .issue_draft
        .configure_prompt(state.selected_panel == PanelFocus::NewIssue);
    state
        .issue_creator_panel
        .set_focused(state.selected_panel == PanelFocus::NewIssue);
    state
        .running_issue_panel
        .set_focused(state.selected_panel == PanelFocus::Running);
    state
        .user_unowned_issue_panel
        .set_focused(state.selected_panel == PanelFocus::UserOwned);
    state
        .completed_issue_panel
        .set_focused(state.selected_panel == PanelFocus::Completed);
}

fn handle_issue_draft_key(key: KeyEvent, state: &mut DashboardState) -> Option<IssueSubmission> {
    if state.issue_draft.is_submitting {
        if is_issue_submit_key(key) {
            state.issue_draft.info_message =
                Some("Issue submission already in progress.".to_string());
        }
        return None;
    }

    if is_alt_char_key(key, 'a') {
        state.issue_draft.cycle_assignee(true);
        state.selected_panel = PanelFocus::NewIssue;
        return None;
    }

    if is_issue_submit_key(key) {
        state.selected_panel = PanelFocus::NewIssue;
        return attempt_issue_submit(state);
    }

    if state.issue_draft.prompt.input(key) {
        state.issue_draft.note_edit();
        state.selected_panel = PanelFocus::NewIssue;
    }

    None
}

fn handle_status_panel_key(key: KeyEvent, state: &mut DashboardState) -> bool {
    let size = match state.last_frame_size {
        Some(size) => size,
        None => return false,
    };

    let layout = dashboard_layout(size);
    let panels = issue_panel_layout(layout.issue_sections);

    match state.selected_panel {
        PanelFocus::UserOwned => {
            let Some(area) = panels.user_owned else {
                return false;
            };
            let lines = issue_line_lines(
                &state.user_unowned_issue_lines.rows,
                "No open issues assigned to you",
                false,
            );
            let (content_len, view_height) = panel_scroll_metrics(area, &lines);
            if view_height == 0 {
                return false;
            }
            matches!(
                state
                    .user_unowned_issue_panel
                    .handle_key_event(key, content_len, view_height,),
                PanelEvent::Scrolled
            )
        }
        PanelFocus::Running => {
            let lines = issue_line_lines(&state.issue_lines.rows, "No issues found", true);
            let (content_len, view_height) = panel_scroll_metrics(panels.running, &lines);
            if view_height == 0 {
                return false;
            }
            matches!(
                state
                    .running_issue_panel
                    .handle_key_event(key, content_len, view_height,),
                PanelEvent::Scrolled
            )
        }
        PanelFocus::Completed => {
            let rows = completed_issue_rows(&state.completed_issue_lines);
            let lines = issue_line_lines(&rows, "No completed issues", true);
            let (content_len, view_height) = panel_scroll_metrics(panels.completed, &lines);
            if view_height == 0 {
                return false;
            }
            matches!(
                state
                    .completed_issue_panel
                    .handle_key_event(key, content_len, view_height,),
                PanelEvent::Scrolled
            )
        }
        PanelFocus::NewIssue => false,
    }
}

fn attempt_issue_submit(state: &mut DashboardState) -> Option<IssueSubmission> {
    if state.issue_draft.is_submitting {
        state.issue_draft.info_message = Some("Issue submission already in progress.".to_string());
        state.issue_draft.validation_error = None;
        return None;
    }

    let prompt = state.issue_draft.prompt_text();
    let prompt = prompt.trim();
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
            let focused = state.selected_panel == PanelFocus::NewIssue;
            state.issue_draft.clear_prompt(focused);
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
    creator: &str,
) -> Result<IssueId> {
    let assignee = submission.assignee.trim();
    let assignee = if assignee.is_empty() {
        None
    } else {
        Some(assignee.to_string())
    };
    let token = auth::read_auth_token()?;
    let creator = User::new(Username::from(creator), token);

    let request = UpsertIssueRequest::new(
        Issue::new(
            IssueType::Task,
            submission.prompt.trim().to_string(),
            creator,
            String::new(),
            IssueStatus::Open,
            assignee,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
        None,
    );

    let response = client
        .create_issue(&request)
        .await
        .context("failed to create issue")?;
    Ok(response.issue_id)
}

fn render(frame: &mut Frame, state: &mut DashboardState) {
    let layout = dashboard_layout(frame.area());
    render_dashboard_header(frame, layout.header, &state.username);
    render_issue_creator(frame, layout.issue_creator, state);
    render_issue_sections(frame, layout.issue_sections, state);
}

fn render_dashboard_header(frame: &mut Frame, area: ratatui::layout::Rect, username: &str) {
    let title = dashboard_title(username);
    let hint = "Tab/Shift+Tab to change panels, j/k or Up/Down to scroll, Ctrl+C to exit.";
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Max(hint.len() as u16)])
        .split(area);

    let title_line = Line::from(Span::styled(
        title,
        Style::default().add_modifier(Modifier::BOLD),
    ));
    let hint_line = Line::from(Span::styled(hint, Style::default().fg(Color::DarkGray)));

    frame.render_widget(Paragraph::new(title_line), chunks[0]);
    frame.render_widget(
        Paragraph::new(hint_line).alignment(Alignment::Right),
        chunks[1],
    );
}

fn dashboard_title(username: &str) -> String {
    let trimmed = username.trim();
    if trimmed.is_empty() {
        "Metis Dashboard".to_string()
    } else {
        format!("Metis Dashboard — {trimmed}")
    }
}

fn render_issue_sections(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    state: &mut DashboardState,
) {
    let panels = issue_panel_layout(area);

    if let Some(rect) = panels.user_owned {
        let title = issue_list_title("Your Issues", &state.user_unowned_issue_lines);
        let lines = issue_line_lines(
            &state.user_unowned_issue_lines.rows,
            "No open issues assigned to you",
            false,
        );
        let panel = Panel::new(Line::from(title), lines);
        frame.render_stateful_widget(panel, rect, &mut state.user_unowned_issue_panel);
    }

    let running_title = issue_list_title("Running issues", &state.issue_lines);
    let running_lines = issue_line_lines(&state.issue_lines.rows, "No issues found", true);
    let running_panel = Panel::new(Line::from(running_title), running_lines);
    frame.render_stateful_widget(
        running_panel,
        panels.running,
        &mut state.running_issue_panel,
    );

    let completed_title = completed_issue_list_title(&state.completed_issue_lines);
    let completed_rows = completed_issue_rows(&state.completed_issue_lines);
    let completed_lines = issue_line_lines(&completed_rows, "No completed issues", true);
    let completed_panel = Panel::new(Line::from(completed_title), completed_lines);
    frame.render_stateful_widget(
        completed_panel,
        panels.completed,
        &mut state.completed_issue_panel,
    );
}

fn render_issue_creator(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    state: &mut DashboardState,
) {
    frame.render_stateful_widget(
        Panel::new("New issue", Vec::new()),
        area,
        &mut state.issue_creator_panel,
    );

    let sections = issue_creator_layout(area);
    let draft = &state.issue_draft;
    frame.render_widget(draft.prompt.widget(), sections.prompt_input);
    render_panel_scrollbar(
        frame,
        sections.prompt_input,
        state.issue_draft_scroll.scrollbar_state,
    );

    let assignee = draft.selected_assignee().unwrap_or("pm");
    let assignee_line = Line::from(vec![
        Span::styled("Assignee: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(format!("@{assignee}"), Style::default().fg(Color::Yellow)),
    ]);
    let assignee_width = assignee_line.width().min(sections.footer.width as usize) as u16;

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
    } else {
        Line::from("")
    };
    let footer_columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(assignee_width), Constraint::Min(0)])
        .split(sections.footer);
    frame.render_widget(Paragraph::new(assignee_line), footer_columns[0]);
    frame.render_widget(
        Paragraph::new(footer).alignment(Alignment::Right),
        footer_columns[1],
    );
}

fn issue_list_title(title: &str, issue_lines: &IssueLines) -> String {
    format!("{title} ({})", issue_lines.rows.len())
}

fn completed_issue_list_title(completed_issue_lines: &CompletedIssueLines) -> String {
    format!(
        "Completed Issues ({})",
        completed_issue_count(completed_issue_lines)
    )
}

fn completed_issue_count(completed_issue_lines: &CompletedIssueLines) -> usize {
    completed_issue_lines.roots.len()
        + completed_issue_lines
            .descendants
            .values()
            .map(Vec::len)
            .sum::<usize>()
}

fn completed_issue_rows(completed_issue_lines: &CompletedIssueLines) -> Vec<IssueLine> {
    let mut rows = Vec::new();
    for root in &completed_issue_lines.roots {
        rows.push(root.clone());
        if let Some(descendants) = completed_issue_descendants(completed_issue_lines, &root.id) {
            rows.extend(descendants.iter().cloned());
        }
    }
    rows
}

struct DashboardLayout {
    header: Rect,
    issue_creator: Rect,
    issue_sections: Rect,
}

fn dashboard_layout(area: Rect) -> DashboardLayout {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(ISSUE_CREATOR_PANEL_HEIGHT),
            Constraint::Min(12),
        ])
        .split(area);

    DashboardLayout {
        header: chunks[0],
        issue_creator: chunks[1],
        issue_sections: chunks[2],
    }
}

struct IssuePanelLayout {
    user_owned: Option<Rect>,
    running: Rect,
    completed: Rect,
}

struct IssueCreatorLayout {
    prompt_input: Rect,
    footer: Rect,
}

fn issue_panel_layout(area: Rect) -> IssuePanelLayout {
    let panels = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(USER_ISSUES_PANEL_HEIGHT),
            Constraint::Fill(1),
            Constraint::Fill(1),
        ])
        .split(area);
    IssuePanelLayout {
        user_owned: Some(panels[0]),
        running: panels[1],
        completed: panels[2],
    }
}

fn issue_creator_layout(area: Rect) -> IssueCreatorLayout {
    let inner = panel_content_area(area);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(inner);

    IssueCreatorLayout {
        prompt_input: sections[0],
        footer: sections[1],
    }
}

fn handle_mouse_scroll(mouse: MouseEvent, state: &mut DashboardState) -> bool {
    if !matches!(
        mouse.kind,
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
    ) {
        return false;
    }

    let hovered_panel = match panel_at_position(state, mouse.column, mouse.row) {
        Some(panel) => panel,
        None => return false,
    };

    let size = match state.last_frame_size {
        Some(size) => size,
        None => return false,
    };
    let layout = dashboard_layout(size);
    let panels = issue_panel_layout(layout.issue_sections);

    match hovered_panel {
        PanelFocus::UserOwned => {
            let Some(rect) = panels.user_owned else {
                return false;
            };
            let lines = issue_line_lines(
                &state.user_unowned_issue_lines.rows,
                "No open issues assigned to you",
                false,
            );
            let (content_len, view_height) = panel_scroll_metrics(rect, &lines);
            matches!(
                state
                    .user_unowned_issue_panel
                    .handle_mouse_event(mouse, content_len, view_height),
                PanelEvent::Scrolled
            )
        }
        PanelFocus::Running => {
            let lines = issue_line_lines(&state.issue_lines.rows, "No issues found", true);
            let (content_len, view_height) = panel_scroll_metrics(panels.running, &lines);
            matches!(
                state
                    .running_issue_panel
                    .handle_mouse_event(mouse, content_len, view_height),
                PanelEvent::Scrolled
            )
        }
        PanelFocus::Completed => {
            let rows = completed_issue_rows(&state.completed_issue_lines);
            let lines = issue_line_lines(&rows, "No completed issues", true);
            let (content_len, view_height) = panel_scroll_metrics(panels.completed, &lines);
            matches!(
                state
                    .completed_issue_panel
                    .handle_mouse_event(mouse, content_len, view_height),
                PanelEvent::Scrolled
            )
        }
        PanelFocus::NewIssue => false,
    }
}

fn handle_mouse_click(mouse: MouseEvent, state: &mut DashboardState) -> bool {
    if !matches!(
        mouse.kind,
        MouseEventKind::Down(_) | MouseEventKind::Up(_) | MouseEventKind::Drag(_)
    ) {
        return false;
    }

    let hovered_panel = match panel_at_position(state, mouse.column, mouse.row) {
        Some(panel) => panel,
        None => return false,
    };

    if state.selected_panel != hovered_panel {
        state.selected_panel = hovered_panel;
        update_panel_focus(state);
    }

    true
}

fn rect_contains(rect: Rect, column: u16, row: u16) -> bool {
    column >= rect.x
        && column < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

fn panel_at_position(state: &DashboardState, column: u16, row: u16) -> Option<PanelFocus> {
    let size = state.last_frame_size?;
    let layout = dashboard_layout(size);
    if rect_contains(layout.issue_creator, column, row) {
        return Some(PanelFocus::NewIssue);
    }

    let panels = issue_panel_layout(layout.issue_sections);
    if let Some(rect) = panels.user_owned {
        if rect_contains(rect, column, row) {
            return Some(PanelFocus::UserOwned);
        }
    }

    if rect_contains(panels.running, column, row) {
        return Some(PanelFocus::Running);
    }

    if rect_contains(panels.completed, column, row) {
        return Some(PanelFocus::Completed);
    }

    None
}

fn clamp_issue_scrolls(state: &mut DashboardState) {
    let size = match state.last_frame_size {
        Some(size) => size,
        None => return,
    };

    let layout = dashboard_layout(size);
    let panels = issue_panel_layout(layout.issue_sections);

    if let Some(rect) = panels.user_owned {
        let lines = issue_line_lines(
            &state.user_unowned_issue_lines.rows,
            "No open issues assigned to you",
            false,
        );
        let (content_len, view_height) = panel_scroll_metrics(rect, &lines);
        state
            .user_unowned_issue_panel
            .sync_scroll(content_len, view_height);
    } else {
        state.user_unowned_issue_panel.sync_scroll(0, 0);
    }

    let running_lines = issue_line_lines(&state.issue_lines.rows, "No issues found", true);
    let (running_len, running_view_height) = panel_scroll_metrics(panels.running, &running_lines);
    state
        .running_issue_panel
        .sync_scroll(running_len, running_view_height);

    let completed_rows = completed_issue_rows(&state.completed_issue_lines);
    let completed_lines = issue_line_lines(&completed_rows, "No completed issues", true);
    let (completed_len, completed_view_height) =
        panel_scroll_metrics(panels.completed, &completed_lines);
    state
        .completed_issue_panel
        .sync_scroll(completed_len, completed_view_height);

    let creator_layout = issue_creator_layout(layout.issue_creator);
    let prompt_view_height = creator_layout.prompt_input.height as usize;
    let prompt_lines = state.issue_draft.prompt.lines().len();
    let cursor_row = state.issue_draft.prompt.cursor().0;
    let max_offset = max_scroll_offset(prompt_lines, prompt_view_height);
    state.issue_draft_scroll.offset = next_scroll_top(
        state.issue_draft_scroll.offset,
        cursor_row,
        prompt_view_height,
    )
    .min(max_offset);
    state.issue_draft_scroll.scrollbar_state = list_scrollbar_state(
        prompt_lines,
        prompt_view_height,
        state.issue_draft_scroll.offset,
    );
}

fn completed_issue_descendants<'a>(
    completed_issue_lines: &'a CompletedIssueLines,
    root_id: &str,
) -> Option<&'a Vec<IssueLine>> {
    completed_issue_lines
        .descendants
        .iter()
        .find_map(|(id, descendants)| {
            if id.to_string() == root_id {
                Some(descendants)
            } else {
                None
            }
        })
}

fn issue_line_lines(
    issue_lines: &[IssueLine],
    empty_message: &str,
    show_hierarchy: bool,
) -> Vec<Line<'static>> {
    if issue_lines.is_empty() {
        return vec![Line::from(Span::styled(
            empty_message.to_string(),
            Style::default().fg(Color::DarkGray),
        ))];
    }

    issue_lines
        .iter()
        .map(|line| {
            let mut spans = Vec::new();
            if show_hierarchy {
                spans.push(Span::raw(issue_prefix(line.depth)));
                spans.push(Span::raw(" "));
            }
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

            Line::from(spans)
        })
        .collect()
}

fn panel_content_area(area: Rect) -> Rect {
    let inner = area.inner(Margin {
        vertical: 1,
        horizontal: 1,
    });
    if inner.height == 0 {
        return inner;
    }

    Rect {
        height: inner.height.saturating_sub(1),
        ..inner
    }
}

fn panel_scroll_metrics(area: Rect, lines: &[Line]) -> (usize, usize) {
    let content_area = panel_content_area(area);
    let view_height = content_area.height as usize;
    let content_len = wrapped_content_len(lines, content_area.width);
    (content_len, view_height)
}

fn max_scroll_offset(total_items: usize, view_height: usize) -> usize {
    if view_height == 0 {
        return 0;
    }
    total_items.saturating_sub(view_height)
}

fn next_scroll_top(prev_top: usize, cursor_row: usize, view_height: usize) -> usize {
    if view_height == 0 {
        return 0;
    }
    if cursor_row < prev_top {
        cursor_row
    } else if prev_top.saturating_add(view_height) <= cursor_row {
        cursor_row.saturating_add(1).saturating_sub(view_height)
    } else {
        prev_top
    }
}

fn list_scrollbar_state(
    total_items: usize,
    view_height: usize,
    scroll_offset: usize,
) -> ScrollbarState {
    let max_offset = max_scroll_offset(total_items, view_height);
    let position = scroll_offset.min(max_offset);
    ScrollbarState::new(total_items)
        .position(position)
        .viewport_content_length(view_height)
}

fn render_panel_scrollbar(frame: &mut Frame, area: Rect, scrollbar_state: ScrollbarState) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let mut state = scrollbar_state;
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .thumb_style(Style::default().fg(Color::White))
        .track_style(Style::default().fg(Color::DarkGray));
    frame.render_stateful_widget(scrollbar, area, &mut state);
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
    let previous_user_unowned_issue_lines = state.user_unowned_issue_lines.clone();
    let previous_completed_issue_lines = state.completed_issue_lines.clone();
    let previous_assignee_options = state.issue_draft.assignees.clone();
    let previous_assignee_index = state.issue_draft.assignee_index;

    let issue_lines = build_issue_lines(&state.issues, &state.jobs, true);
    let user_unowned_issue_lines =
        build_user_unowned_issue_lines(&state.username, &state.issues, &state.jobs);
    let completed_issue_lines = build_completed_issue_lines(&state.issues, &state.jobs);
    update_assignee_options(state);

    state.issue_lines = issue_lines;
    state.user_unowned_issue_lines = user_unowned_issue_lines;
    state.completed_issue_lines = completed_issue_lines;
    update_panel_focus(state);
    clamp_issue_scrolls(state);

    previous_issue_lines != state.issue_lines
        || previous_user_unowned_issue_lines != state.user_unowned_issue_lines
        || previous_completed_issue_lines != state.completed_issue_lines
        || previous_assignee_options != state.issue_draft.assignees
        || previous_assignee_index != state.issue_draft.assignee_index
}

fn update_assignee_options(state: &mut DashboardState) {
    let fallback = "pm";
    let preferred = state
        .issue_draft
        .selected_assignee()
        .unwrap_or(fallback)
        .to_string();
    let options = build_assignee_options(&state.issues);
    if options != state.issue_draft.assignees {
        state.issue_draft.assignees = options;
    }

    let next_index = state
        .issue_draft
        .assignees
        .iter()
        .position(|assignee| assignee == &preferred)
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

fn build_user_unowned_issue_lines(
    username: &str,
    issues: &[IssueRecord],
    jobs: &[JobDetails],
) -> IssueLines {
    let assigned: Vec<IssueRecord> = issues
        .iter()
        .filter(|issue| issue.assignee.as_deref() == Some(username))
        .filter(|issue| issue.status == IssueStatus::Open)
        .cloned()
        .collect();

    let mut lines = build_issue_lines(&assigned, jobs, false);
    for row in &mut lines.rows {
        row.depth = 0;
    }
    lines
}

fn build_issue_lines(
    issues: &[IssueRecord],
    jobs: &[JobDetails],
    exclude_inactive_roots: bool,
) -> IssueLines {
    let nodes = build_issue_nodes(issues, jobs);

    let mut roots: Vec<IssueId> = nodes
        .iter()
        .filter(|(_, node)| node.parent.is_none())
        .map(|(id, _)| id.clone())
        .collect();
    roots.sort_by(|a, b| compare_issue_nodes(&nodes, a, b));

    let mut rows = Vec::new();
    let mut visited: HashSet<IssueId> = HashSet::new();
    for root in roots {
        if exclude_inactive_roots {
            if let Some(node) = nodes.get(&root) {
                if matches!(
                    node.record.status,
                    IssueStatus::Closed | IssueStatus::Dropped
                ) {
                    continue;
                }
            }
        }
        append_issue(&root, 0, &mut rows, &mut visited, &nodes);
    }

    IssueLines { rows }
}

fn build_issue_nodes(issues: &[IssueRecord], jobs: &[JobDetails]) -> HashMap<IssueId, IssueNode> {
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

    nodes
}

fn build_completed_issue_lines(issues: &[IssueRecord], jobs: &[JobDetails]) -> CompletedIssueLines {
    let nodes = build_issue_nodes(issues, jobs);
    if nodes.is_empty() {
        return CompletedIssueLines::default();
    }

    let mut roots: Vec<IssueId> = nodes
        .iter()
        .filter(|(_, node)| node.parent.is_none())
        .map(|(id, _)| id.clone())
        .collect();
    roots.sort_by(|a, b| compare_issue_nodes(&nodes, a, b));

    let mut completed_roots = Vec::new();
    let mut completed_descendants = HashMap::new();

    for root in roots {
        if !is_completed_tree(&root, &nodes, &mut HashSet::new()) {
            continue;
        }

        let mut lines = Vec::new();
        let mut visited = HashSet::new();
        collect_issue_lines(&root, 0, &mut lines, &mut visited, &nodes);
        if let Some(root_line) = lines.first().cloned() {
            completed_roots.push(root_line);
        }
        if lines.len() > 1 {
            completed_descendants.insert(root.clone(), lines[1..].to_vec());
        } else {
            completed_descendants.insert(root.clone(), Vec::new());
        }
    }

    CompletedIssueLines {
        roots: completed_roots,
        descendants: completed_descendants,
    }
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

fn collect_issue_lines(
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
        collect_issue_lines(&child, depth + 1, rows, visited, nodes);
    }
}

fn is_completed_tree(
    id: &IssueId,
    nodes: &HashMap<IssueId, IssueNode>,
    visited: &mut HashSet<IssueId>,
) -> bool {
    if !visited.insert(id.clone()) {
        return false;
    }

    let Some(node) = nodes.get(id) else {
        return false;
    };

    if !matches!(
        node.record.status,
        IssueStatus::Closed | IssueStatus::Dropped
    ) {
        return false;
    }

    node.children
        .iter()
        .all(|child| is_completed_tree(child, nodes, visited))
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
        _ => 4,
    }
}

fn issue_status_order(status: IssueStatus) -> usize {
    match status {
        IssueStatus::InProgress => 0,
        IssueStatus::Open => 1,
        IssueStatus::Dropped => 2,
        IssueStatus::Closed => 3,
        _ => 4,
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
        other => format!("error: {other:?}"),
    }
}

fn status_style(status: Status) -> Style {
    match status {
        Status::Complete => Style::default().fg(Color::Green),
        Status::Running => Style::default().fg(Color::Yellow),
        Status::Failed => Style::default().fg(Color::Red),
        Status::Pending => Style::default().fg(Color::Blue),
        _ => Style::default(),
    }
}

fn issue_status_style(status: IssueStatus) -> Style {
    match status {
        IssueStatus::Open => Style::default().fg(Color::Blue),
        IssueStatus::InProgress => Style::default().fg(Color::Yellow),
        IssueStatus::Closed => Style::default().fg(Color::Green),
        IssueStatus::Dropped => Style::default().fg(Color::Rgb(139, 0, 0)),
        _ => Style::default(),
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
        _ => ("unknown".to_string(), Style::default()),
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
    use crate::client::MetisClient;
    use crate::test_utils::ids::{issue_id, task_id};
    use chrono::Duration as ChronoDuration;
    use crossterm::event::{
        Event as CrosstermEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use httpmock::prelude::*;
    use metis_common::issues::UpsertIssueResponse;
    use metis_common::jobs::{BundleSpec, Task};
    use metis_common::task_status::Event;
    use ratatui::buffer::Buffer;
    use ratatui::prelude::StatefulWidget;
    use serde_json::json;
    use std::collections::HashMap;
    use std::{env, fs};
    use tempfile::tempdir;

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
            other => unreachable!("unsupported task status variant: {other:?}"),
        }

        JobRecord::new(
            task_id(id),
            Task::new("0".into(), BundleSpec::None, None, None, HashMap::new()),
            None,
            log,
        )
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

    fn child_of(issue_ref: &str) -> IssueDependency {
        IssueDependency::new(IssueDependencyType::ChildOf, issue_id(issue_ref))
    }

    fn blocked_on(issue_ref: &str) -> IssueDependency {
        IssueDependency::new(IssueDependencyType::BlockedOn, issue_id(issue_ref))
    }

    #[test]
    fn dashboard_title_includes_username_when_present() {
        assert_eq!(dashboard_title("cprussin"), "Metis Dashboard — cprussin");
    }

    #[test]
    fn dashboard_title_skips_username_when_blank() {
        assert_eq!(dashboard_title(" "), "Metis Dashboard");
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
            issue("i-3", IssueStatus::Closed, vec![child_of("i-1")]),
            issue("i-2", IssueStatus::InProgress, vec![]),
        ];

        let lines = build_issue_lines(&issues, &[], false);

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
                vec![blocked_on("i-closed"), blocked_on("i-open")],
            ),
            issue("i-closed", IssueStatus::Closed, vec![]),
            issue("i-open", IssueStatus::Open, vec![]),
        ];

        let lines = build_issue_lines(&issues, &[], false);

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
        let lines = build_issue_lines(&issues, &[], false);

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

        let lines = build_issue_lines(&issues, &jobs, false);

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

        let lines = build_issue_lines(&issues, &[], false);

        let line = lines.rows.first().expect("issue line missing");
        assert_eq!(line.summary, "investigate logs");
        assert_eq!(line.progress.as_deref(), Some("drafting tests"));
    }

    fn dashboard_state_with_issues(issue_count: usize) -> DashboardState {
        let issues: Vec<IssueRecord> = (0..issue_count)
            .map(|index| issue(&format!("i-{index}"), IssueStatus::Open, Vec::new()))
            .collect();
        DashboardState {
            issue_lines: build_issue_lines(&issues, &[], false),
            user_unowned_issue_lines: build_issue_lines(&issues, &[], false),
            ..DashboardState::default()
        }
    }

    #[test]
    fn mouse_scroll_targets_hovered_panel_without_changing_focus() {
        let mut state = dashboard_state_with_issues(12);
        let size = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 40,
        };
        state.last_frame_size = Some(size);
        state.selected_panel = PanelFocus::Completed;
        update_panel_focus(&mut state);

        let layout = dashboard_layout(size);
        let panels = issue_panel_layout(layout.issue_sections);
        let mouse = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: panels.running.x.saturating_add(1),
            row: panels.running.y.saturating_add(1),
            modifiers: KeyModifiers::NONE,
        };

        let handled = handle_mouse_scroll(mouse, &mut state);

        assert!(handled);
        assert_eq!(state.selected_panel, PanelFocus::Completed);
        assert_eq!(state.running_issue_panel.scroll_offset(), 1);
        assert_eq!(state.completed_issue_panel.scroll_offset(), 0);
    }

    #[test]
    fn mouse_scroll_ignored_without_hovered_panel() {
        let mut state = dashboard_state_with_issues(12);
        let size = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 40,
        };
        state.last_frame_size = Some(size);
        state.selected_panel = PanelFocus::Running;
        update_panel_focus(&mut state);

        let mouse = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: size.width.saturating_add(1),
            row: size.height.saturating_add(1),
            modifiers: KeyModifiers::NONE,
        };

        let handled = handle_mouse_scroll(mouse, &mut state);

        assert!(!handled);
        assert_eq!(state.running_issue_panel.scroll_offset(), 0);
        assert_eq!(state.selected_panel, PanelFocus::Running);
    }

    #[test]
    fn in_progress_issues_show_waiting_when_children_open() {
        let issues = vec![
            issue(
                "i-parent",
                IssueStatus::InProgress,
                vec![child_of("i-root")],
            ),
            issue("i-root", IssueStatus::InProgress, vec![child_of("i-grand")]),
            issue("i-grand", IssueStatus::Open, vec![]),
        ];

        let lines = build_issue_lines(&issues, &[], false);

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
    fn completed_issue_lines_include_closed_roots_with_closed_descendants() {
        let issues = vec![
            issue("i-root", IssueStatus::Closed, vec![]),
            issue("i-child", IssueStatus::Closed, vec![child_of("i-root")]),
            issue("i-root-open", IssueStatus::Open, vec![]),
            issue(
                "i-child-open",
                IssueStatus::Open,
                vec![child_of("i-root-closed-with-open-child")],
            ),
            issue("i-root-closed-with-open-child", IssueStatus::Closed, vec![]),
        ];

        let lines = build_completed_issue_lines(&issues, &[]);

        assert_eq!(lines.roots.len(), 1);
        assert_eq!(lines.roots[0].id, issue_id("i-root").to_string());

        let descendants = lines
            .descendants
            .get(&issue_id("i-root"))
            .expect("missing descendants");
        assert_eq!(descendants.len(), 1);
        assert_eq!(descendants[0].id, issue_id("i-child").to_string());
        assert_eq!(descendants[0].depth, 1);
    }

    #[test]
    fn completed_issue_lines_include_dropped_trees() {
        let issues = vec![
            issue("i-closed", IssueStatus::Closed, vec![]),
            issue("i-dropped-root", IssueStatus::Dropped, vec![]),
            issue(
                "i-dropped-child",
                IssueStatus::Dropped,
                vec![child_of("i-dropped-root")],
            ),
            issue("i-open", IssueStatus::Open, vec![]),
        ];

        let lines = build_completed_issue_lines(&issues, &[]);

        assert_eq!(lines.roots.len(), 2);
        assert!(lines
            .roots
            .iter()
            .any(|line| line.id == issue_id("i-closed").to_string()));
        assert!(lines
            .roots
            .iter()
            .any(|line| line.id == issue_id("i-dropped-root").to_string()));

        let descendants = lines
            .descendants
            .get(&issue_id("i-dropped-root"))
            .expect("missing descendants");
        assert_eq!(descendants.len(), 1);
        assert_eq!(descendants[0].id, issue_id("i-dropped-child").to_string());
        assert_eq!(descendants[0].depth, 1);
    }

    #[test]
    fn completed_issue_lines_track_nested_depth() {
        let issues = vec![
            issue("i-root", IssueStatus::Closed, vec![]),
            issue("i-child", IssueStatus::Closed, vec![child_of("i-root")]),
            issue(
                "i-grandchild",
                IssueStatus::Closed,
                vec![child_of("i-child")],
            ),
        ];

        let lines = build_completed_issue_lines(&issues, &[]);

        let descendants = lines
            .descendants
            .get(&issue_id("i-root"))
            .expect("missing descendants");
        assert_eq!(descendants.len(), 2);
        assert_eq!(descendants[0].depth, 1);
        assert_eq!(descendants[1].depth, 2);
    }

    #[test]
    fn completed_issue_list_title_counts_descendants() {
        let issues = vec![
            issue("i-root", IssueStatus::Closed, vec![]),
            issue("i-child", IssueStatus::Closed, vec![child_of("i-root")]),
            issue(
                "i-grandchild",
                IssueStatus::Closed,
                vec![child_of("i-child")],
            ),
        ];

        let lines = build_completed_issue_lines(&issues, &[]);

        assert_eq!(completed_issue_list_title(&lines), "Completed Issues (3)");
    }

    #[test]
    fn completed_issue_rows_include_descendants_after_root() {
        let issues = vec![
            issue("i-root", IssueStatus::Closed, vec![]),
            issue("i-child", IssueStatus::Closed, vec![child_of("i-root")]),
        ];

        let lines = build_completed_issue_lines(&issues, &[]);
        let rows = completed_issue_rows(&lines);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, issue_id("i-root").to_string());
        assert_eq!(rows[1].id, issue_id("i-child").to_string());
        assert_eq!(rows[1].depth, 1);
    }

    #[test]
    fn user_unowned_issue_lines_empty_without_username() {
        let issues = vec![issue_with_assignee(
            "i-open",
            IssueStatus::Open,
            Some("alice"),
        )];

        let lines = build_user_unowned_issue_lines("", &issues, &[]);

        assert!(lines.rows.is_empty());
    }

    #[test]
    fn user_unowned_issue_lines_include_user_assignee() {
        let issues = vec![
            issue_with_assignee("i-open", IssueStatus::Open, Some("alice")),
            issue_with_assignee("i-other", IssueStatus::Open, Some("bot")),
        ];
        let lines = build_user_unowned_issue_lines("alice", &issues, &[]);

        assert_eq!(lines.rows.len(), 1);
        assert_eq!(lines.rows[0].id, issue_id("i-open").to_string());
    }

    #[test]
    fn user_unowned_issue_lines_only_show_open() {
        let issues = vec![
            issue_with_assignee("i-open", IssueStatus::Open, Some("alice")),
            issue_with_assignee("i-progress", IssueStatus::InProgress, Some("alice")),
            issue_with_assignee("i-closed", IssueStatus::Closed, Some("alice")),
        ];
        let lines = build_user_unowned_issue_lines("alice", &issues, &[]);

        assert_eq!(lines.rows.len(), 1);
        assert_eq!(lines.rows[0].id, issue_id("i-open").to_string());
    }

    #[test]
    fn user_unowned_issue_lines_flatten_depth() {
        let issues = vec![
            IssueRecord {
                id: issue_id("i-root"),
                description: "i-root".to_string(),
                progress: String::new(),
                status: IssueStatus::Open,
                assignee: Some("alice".to_string()),
                dependencies: Vec::new(),
            },
            IssueRecord {
                id: issue_id("i-child"),
                description: "i-child".to_string(),
                progress: String::new(),
                status: IssueStatus::Open,
                assignee: Some("alice".to_string()),
                dependencies: vec![child_of("i-root")],
            },
        ];

        let lines = build_user_unowned_issue_lines("alice", &issues, &[]);

        assert_eq!(lines.rows.len(), 2);
        assert!(lines.rows.iter().all(|line| line.depth == 0));
    }

    #[test]
    fn running_issue_lines_include_user_and_agent_assignments() {
        let issues = vec![
            issue_with_assignee("i-user", IssueStatus::Open, Some("alice")),
            issue_with_assignee("i-agent", IssueStatus::Open, Some("bot")),
        ];
        let mut state = DashboardState {
            username: "alice".to_string(),
            issues,
            ..Default::default()
        };

        update_views(&mut state);

        assert_eq!(state.issue_lines.rows.len(), 2);
        assert!(state
            .issue_lines
            .rows
            .iter()
            .any(|line| line.id == issue_id("i-user").to_string()));
        assert!(state
            .issue_lines
            .rows
            .iter()
            .any(|line| line.id == issue_id("i-agent").to_string()));
    }

    #[test]
    fn running_issue_lines_exclude_closed_root_tree() {
        let issues = vec![
            issue("i-closed-root", IssueStatus::Closed, vec![]),
            issue(
                "i-child-open",
                IssueStatus::Open,
                vec![child_of("i-closed-root")],
            ),
            issue("i-open-root", IssueStatus::Open, vec![]),
        ];

        let lines = build_issue_lines(&issues, &[], true);

        assert_eq!(lines.rows.len(), 1);
        assert_eq!(lines.rows[0].id, issue_id("i-open-root").to_string());
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
    fn update_assignee_options_keeps_pm_as_default() {
        let mut state = DashboardState {
            issues: vec![issue_with_assignee("i-1", IssueStatus::Open, Some("alice"))],
            ..DashboardState::default()
        };

        update_assignee_options(&mut state);

        assert_eq!(state.issue_draft.selected_assignee(), Some("pm"));
    }

    #[test]
    fn attempt_issue_submit_requires_prompt() {
        let mut state = DashboardState::default();
        state.issue_draft.set_prompt("   ", true);

        let submission = attempt_issue_submit(&mut state);

        assert!(submission.is_none());
        assert_eq!(
            state.issue_draft.validation_error.as_deref(),
            Some("Prompt cannot be empty.")
        );
        assert!(!state.issue_draft.is_submitting);
    }

    #[test]
    fn attempt_issue_submit_rejects_whitespace_only_with_newlines() {
        let mut state = DashboardState::default();
        state.issue_draft.set_prompt("\n  \n", true);

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
        state.issue_draft.set_prompt("Ship dashboard", true);
        state.issue_draft.assignees = vec!["pm".to_string()];

        let submission = attempt_issue_submit(&mut state).expect("submission missing");

        assert_eq!(submission.prompt, "Ship dashboard");
        assert_eq!(submission.assignee, "pm");
        assert!(state.issue_draft.is_submitting);
    }

    #[test]
    fn issue_submission_success_preserves_selected_panel() {
        let mut state = DashboardState {
            selected_panel: PanelFocus::NewIssue,
            ..Default::default()
        };
        state.issue_draft.set_prompt("Ship dashboard", true);
        state.issue_draft.is_submitting = true;

        handle_issue_submission_result(&mut state, "pm", Ok(issue_id("i-new")));

        assert!(matches!(state.selected_panel, PanelFocus::NewIssue));
        assert!(!state.issue_draft.is_submitting);
    }

    #[test]
    fn alt_a_cycles_assignee_in_new_issue_panel() {
        let mut state = DashboardState::default();
        state.issue_draft.assignees = vec!["pm".to_string(), "alice".to_string()];

        let outcome = handle_event(
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT)),
            &mut state,
        );

        assert!(!outcome.should_quit);
        assert!(outcome.submission.is_none());
        assert_eq!(state.issue_draft.selected_assignee(), Some("alice"));
    }

    #[test]
    fn alt_a_does_not_cycle_assignee_when_not_focused() {
        let mut state = DashboardState {
            selected_panel: PanelFocus::Running,
            ..DashboardState::default()
        };
        state.issue_draft.assignees = vec!["pm".to_string(), "alice".to_string()];

        let outcome = handle_event(
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT)),
            &mut state,
        );

        assert!(!outcome.should_quit);
        assert!(outcome.submission.is_none());
        assert_eq!(state.issue_draft.selected_assignee(), Some("pm"));
    }

    #[test]
    fn tab_switches_focus_away_from_new_issue_panel() {
        let mut state = DashboardState::default();
        update_panel_focus(&mut state);

        let outcome = handle_event(
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
            &mut state,
        );

        assert!(!outcome.should_quit);
        assert!(outcome.submission.is_none());
        assert_eq!(state.selected_panel, PanelFocus::UserOwned);
    }

    #[test]
    fn tab_switches_focus_back_to_new_issue_panel() {
        let mut state = DashboardState {
            selected_panel: PanelFocus::Completed,
            ..DashboardState::default()
        };

        let outcome = handle_event(
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
            &mut state,
        );

        assert!(!outcome.should_quit);
        assert!(outcome.submission.is_none());
        assert_eq!(state.selected_panel, PanelFocus::NewIssue);
    }

    #[test]
    fn shift_tab_moves_focus_to_previous_panel() {
        let mut state = DashboardState {
            selected_panel: PanelFocus::UserOwned,
            ..DashboardState::default()
        };
        update_panel_focus(&mut state);

        let outcome = handle_event(
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT)),
            &mut state,
        );

        assert!(!outcome.should_quit);
        assert!(outcome.submission.is_none());
        assert_eq!(state.selected_panel, PanelFocus::NewIssue);
    }

    #[test]
    fn backtab_moves_focus_to_previous_panel() {
        let mut state = DashboardState {
            selected_panel: PanelFocus::UserOwned,
            ..DashboardState::default()
        };
        update_panel_focus(&mut state);

        let outcome = handle_event(
            CrosstermEvent::Key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE)),
            &mut state,
        );

        assert!(!outcome.should_quit);
        assert!(outcome.submission.is_none());
        assert_eq!(state.selected_panel, PanelFocus::NewIssue);
    }

    #[test]
    fn new_issue_panel_hides_scroll_keybinding() {
        let mut state = DashboardState::default();

        let area = Rect::new(0, 0, 50, 6);
        let mut buffer = Buffer::empty(area);
        let panel = Panel::new("New issue", Vec::new());
        panel.render(area, &mut buffer, &mut state.issue_creator_panel);

        let footer_y = area.y + area.height - 2;
        let footer = row_text(&buffer, footer_y, area.width);
        assert!(!footer.contains("j/k or Up/Down"));
        assert!(footer.contains("Alt+a"));
    }

    #[test]
    fn mouse_scroll_down_updates_running_issue_offset() {
        let issues = (0..25)
            .map(|index| issue(&format!("i-{index}"), IssueStatus::Open, vec![]))
            .collect();
        let mut state = DashboardState {
            issues,
            ..DashboardState::default()
        };
        state.selected_panel = PanelFocus::Completed;
        update_panel_focus(&mut state);
        update_views(&mut state);
        state.last_frame_size = Some(Rect::new(0, 0, 80, 30));

        let layout = dashboard_layout(state.last_frame_size.expect("size missing"));
        let panels = issue_panel_layout(layout.issue_sections);
        let mouse = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: panels.running.x + 1,
            row: panels.running.y + 1,
            modifiers: KeyModifiers::NONE,
        };

        let outcome = handle_event(CrosstermEvent::Mouse(mouse), &mut state);

        assert!(!outcome.should_quit);
        assert!(outcome.submission.is_none());
        assert_eq!(state.running_issue_panel.scroll_offset(), 1);
        assert_eq!(state.selected_panel, PanelFocus::Completed);
        assert!(state.completed_issue_panel.focused());
    }

    #[test]
    fn wrapped_issue_lines_scroll_with_keyboard_input() {
        let long_description = "x".repeat(200);
        let issue = IssueRecord {
            description: long_description,
            ..issue("i-long", IssueStatus::Open, vec![])
        };
        let mut state = DashboardState {
            issues: vec![issue],
            ..DashboardState::default()
        };
        state.selected_panel = PanelFocus::Running;
        state.last_frame_size = Some(Rect::new(0, 0, 30, 30));
        update_views(&mut state);

        let outcome = handle_event(
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            &mut state,
        );

        assert!(!outcome.should_quit);
        assert_eq!(state.running_issue_panel.scroll_offset(), 1);
    }

    #[test]
    fn mouse_scroll_down_updates_user_owned_issue_offset() {
        let issues = (0..10)
            .map(|index| {
                issue_with_assignee(&format!("i-{index}"), IssueStatus::Open, Some("alice"))
            })
            .collect();
        let mut state = DashboardState {
            issues,
            username: "alice".to_string(),
            ..DashboardState::default()
        };
        update_views(&mut state);
        state.last_frame_size = Some(Rect::new(0, 0, 80, 30));

        let layout = dashboard_layout(state.last_frame_size.expect("size missing"));
        let panels = issue_panel_layout(layout.issue_sections);
        let user_owned = panels.user_owned.expect("user-owned panel missing");
        let mouse = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: user_owned.x + 1,
            row: user_owned.y + 1,
            modifiers: KeyModifiers::NONE,
        };

        let outcome = handle_event(CrosstermEvent::Mouse(mouse), &mut state);

        assert!(!outcome.should_quit);
        assert!(outcome.submission.is_none());
        assert_eq!(state.user_unowned_issue_panel.scroll_offset(), 1);
    }

    #[test]
    fn mouse_scroll_down_updates_completed_issue_offset() {
        let issues = (0..10)
            .map(|index| issue(&format!("i-{index}"), IssueStatus::Closed, vec![]))
            .collect();
        let mut state = DashboardState {
            issues,
            ..DashboardState::default()
        };
        update_views(&mut state);
        state.last_frame_size = Some(Rect::new(0, 0, 80, 30));

        let layout = dashboard_layout(state.last_frame_size.expect("size missing"));
        let panels = issue_panel_layout(layout.issue_sections);
        let mouse = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: panels.completed.x + 1,
            row: panels.completed.y + 1,
            modifiers: KeyModifiers::NONE,
        };

        let outcome = handle_event(CrosstermEvent::Mouse(mouse), &mut state);

        assert!(!outcome.should_quit);
        assert!(outcome.submission.is_none());
        assert_eq!(state.completed_issue_panel.scroll_offset(), 1);
    }

    #[test]
    fn mouse_scroll_on_unfocused_panel_routes_to_hovered_panel() {
        let mut issues = (0..25)
            .map(|index| issue(&format!("i-open-{index}"), IssueStatus::Open, vec![]))
            .collect::<Vec<_>>();
        issues.extend(
            (0..25).map(|index| issue(&format!("i-closed-{index}"), IssueStatus::Closed, vec![])),
        );
        let mut state = DashboardState {
            issues,
            ..DashboardState::default()
        };
        state.selected_panel = PanelFocus::Running;
        update_panel_focus(&mut state);
        update_views(&mut state);
        state.last_frame_size = Some(Rect::new(0, 0, 80, 30));

        let layout = dashboard_layout(state.last_frame_size.expect("size missing"));
        let panels = issue_panel_layout(layout.issue_sections);
        let mouse = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: panels.completed.x + 1,
            row: panels.completed.y + 1,
            modifiers: KeyModifiers::NONE,
        };

        let outcome = handle_event(CrosstermEvent::Mouse(mouse), &mut state);

        assert!(!outcome.should_quit);
        assert!(outcome.submission.is_none());
        assert_eq!(state.running_issue_panel.scroll_offset(), 0);
        assert_eq!(state.completed_issue_panel.scroll_offset(), 1);
        assert_eq!(state.selected_panel, PanelFocus::Running);
        assert!(state.running_issue_panel.focused());
        assert!(!state.completed_issue_panel.focused());
    }

    #[test]
    fn mouse_scroll_outside_panels_does_not_scroll_focused_panel() {
        let issues = (0..25)
            .map(|index| issue(&format!("i-{index}"), IssueStatus::Open, vec![]))
            .collect();
        let mut state = DashboardState {
            issues,
            ..DashboardState::default()
        };
        state.selected_panel = PanelFocus::Running;
        update_panel_focus(&mut state);
        update_views(&mut state);
        state.last_frame_size = Some(Rect::new(0, 0, 80, 30));

        let layout = dashboard_layout(state.last_frame_size.expect("size missing"));
        let mouse = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: layout.header.x,
            row: layout.header.y,
            modifiers: KeyModifiers::NONE,
        };

        let outcome = handle_event(CrosstermEvent::Mouse(mouse), &mut state);

        assert!(!outcome.should_quit);
        assert!(outcome.submission.is_none());
        assert_eq!(state.running_issue_panel.scroll_offset(), 0);
        assert_eq!(state.selected_panel, PanelFocus::Running);
        assert!(state.running_issue_panel.focused());
    }

    #[test]
    fn mouse_click_focuses_running_panel() {
        let mut state = DashboardState {
            selected_panel: PanelFocus::NewIssue,
            ..DashboardState::default()
        };
        update_panel_focus(&mut state);
        state.last_frame_size = Some(Rect::new(0, 0, 80, 30));

        let layout = dashboard_layout(state.last_frame_size.expect("size missing"));
        let panels = issue_panel_layout(layout.issue_sections);
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: panels.running.x + 1,
            row: panels.running.y + 1,
            modifiers: KeyModifiers::NONE,
        };

        let outcome = handle_event(CrosstermEvent::Mouse(mouse), &mut state);

        assert!(!outcome.should_quit);
        assert!(outcome.submission.is_none());
        assert_eq!(state.selected_panel, PanelFocus::Running);
        assert!(state.running_issue_panel.focused());
        assert!(!state.issue_creator_panel.focused());
    }

    #[test]
    fn mouse_release_focuses_running_panel() {
        let mut state = DashboardState {
            selected_panel: PanelFocus::NewIssue,
            ..DashboardState::default()
        };
        update_panel_focus(&mut state);
        state.last_frame_size = Some(Rect::new(0, 0, 80, 30));

        let layout = dashboard_layout(state.last_frame_size.expect("size missing"));
        let panels = issue_panel_layout(layout.issue_sections);
        let mouse = MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: panels.running.x + 1,
            row: panels.running.y + 1,
            modifiers: KeyModifiers::NONE,
        };

        let outcome = handle_event(CrosstermEvent::Mouse(mouse), &mut state);

        assert!(!outcome.should_quit);
        assert!(outcome.submission.is_none());
        assert_eq!(state.selected_panel, PanelFocus::Running);
        assert!(state.running_issue_panel.focused());
        assert!(!state.issue_creator_panel.focused());
    }

    #[test]
    fn mouse_click_focuses_issue_creator_panel() {
        let mut state = DashboardState {
            selected_panel: PanelFocus::Running,
            ..DashboardState::default()
        };
        update_panel_focus(&mut state);
        state.last_frame_size = Some(Rect::new(0, 0, 80, 30));

        let layout = dashboard_layout(state.last_frame_size.expect("size missing"));
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: layout.issue_creator.x + 1,
            row: layout.issue_creator.y + 1,
            modifiers: KeyModifiers::NONE,
        };

        let outcome = handle_event(CrosstermEvent::Mouse(mouse), &mut state);

        assert!(!outcome.should_quit);
        assert!(outcome.submission.is_none());
        assert_eq!(state.selected_panel, PanelFocus::NewIssue);
        assert!(state.issue_creator_panel.focused());
        assert!(!state.running_issue_panel.focused());
    }

    #[test]
    fn mouse_click_focuses_user_owned_panel() {
        let mut state = DashboardState {
            selected_panel: PanelFocus::Running,
            ..DashboardState::default()
        };
        update_panel_focus(&mut state);
        state.last_frame_size = Some(Rect::new(0, 0, 80, 30));

        let layout = dashboard_layout(state.last_frame_size.expect("size missing"));
        let panels = issue_panel_layout(layout.issue_sections);
        let user_owned = panels.user_owned.expect("user-owned panel missing");
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: user_owned.x + 1,
            row: user_owned.y + 1,
            modifiers: KeyModifiers::NONE,
        };

        let outcome = handle_event(CrosstermEvent::Mouse(mouse), &mut state);

        assert!(!outcome.should_quit);
        assert!(outcome.submission.is_none());
        assert_eq!(state.selected_panel, PanelFocus::UserOwned);
        assert!(state.user_unowned_issue_panel.focused());
        assert!(!state.running_issue_panel.focused());
    }

    #[test]
    fn mouse_click_focuses_completed_panel() {
        let mut state = DashboardState {
            selected_panel: PanelFocus::UserOwned,
            ..DashboardState::default()
        };
        update_panel_focus(&mut state);
        state.last_frame_size = Some(Rect::new(0, 0, 80, 30));

        let layout = dashboard_layout(state.last_frame_size.expect("size missing"));
        let panels = issue_panel_layout(layout.issue_sections);
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: panels.completed.x + 1,
            row: panels.completed.y + 1,
            modifiers: KeyModifiers::NONE,
        };

        let outcome = handle_event(CrosstermEvent::Mouse(mouse), &mut state);

        assert!(!outcome.should_quit);
        assert!(outcome.submission.is_none());
        assert_eq!(state.selected_panel, PanelFocus::Completed);
        assert!(state.completed_issue_panel.focused());
        assert!(!state.user_unowned_issue_panel.focused());
    }

    #[test]
    fn mouse_click_outside_panels_does_not_change_focus() {
        let mut state = DashboardState {
            selected_panel: PanelFocus::Running,
            ..DashboardState::default()
        };
        update_panel_focus(&mut state);
        state.last_frame_size = Some(Rect::new(0, 0, 80, 30));

        let layout = dashboard_layout(state.last_frame_size.expect("size missing"));
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: layout.header.x,
            row: layout.header.y,
            modifiers: KeyModifiers::NONE,
        };

        let outcome = handle_event(CrosstermEvent::Mouse(mouse), &mut state);

        assert!(!outcome.should_quit);
        assert!(outcome.submission.is_none());
        assert_eq!(state.selected_panel, PanelFocus::Running);
        assert!(state.running_issue_panel.focused());
        assert!(!state.issue_creator_panel.focused());
    }

    #[test]
    fn alt_enter_submits_issue_prompt() {
        let mut state = DashboardState::default();
        state.issue_draft.set_prompt("Ship dashboard", true);
        state.issue_draft.assignees = vec!["pm".to_string()];

        let submission =
            handle_issue_draft_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT), &mut state)
                .expect("submission missing");

        assert_eq!(submission.prompt, "Ship dashboard");
        assert_eq!(submission.assignee, "pm");
        assert!(state.issue_draft.is_submitting);
    }

    #[test]
    fn q_inserts_into_issue_prompt_when_focused() {
        let mut state = DashboardState::default();
        update_panel_focus(&mut state);

        let outcome = handle_event(
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
            &mut state,
        );

        assert!(!outcome.should_quit);
        assert!(outcome.submission.is_none());
        assert_eq!(state.issue_draft.prompt_text(), "q");
    }

    #[test]
    fn shift_q_inserts_into_issue_prompt_when_focused() {
        let mut state = DashboardState::default();
        update_panel_focus(&mut state);

        let outcome = handle_event(
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::SHIFT)),
            &mut state,
        );

        assert!(!outcome.should_quit);
        assert!(outcome.submission.is_none());
        assert_eq!(state.issue_draft.prompt_text(), "Q");
    }

    #[test]
    fn q_does_not_edit_issue_prompt_when_not_focused() {
        let mut state = DashboardState {
            selected_panel: PanelFocus::Running,
            ..DashboardState::default()
        };
        update_panel_focus(&mut state);

        let outcome = handle_event(
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
            &mut state,
        );

        assert!(!outcome.should_quit);
        assert!(outcome.submission.is_none());
        assert!(state.issue_draft.prompt_text().is_empty());
    }

    #[test]
    fn escape_does_not_quit_when_not_focused() {
        let mut state = DashboardState {
            selected_panel: PanelFocus::Running,
            ..DashboardState::default()
        };
        update_panel_focus(&mut state);

        let outcome = handle_event(
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            &mut state,
        );

        assert!(!outcome.should_quit);
        assert!(outcome.submission.is_none());
        assert!(state.issue_draft.prompt_text().is_empty());
    }

    #[test]
    fn escape_does_not_quit_when_focused() {
        let mut state = DashboardState::default();
        update_panel_focus(&mut state);

        let outcome = handle_event(
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            &mut state,
        );

        assert!(!outcome.should_quit);
        assert!(outcome.submission.is_none());
    }

    #[test]
    fn paste_event_inserts_prompt_text() {
        let mut state = DashboardState::default();
        update_panel_focus(&mut state);
        state.issue_draft.validation_error = Some("Prompt cannot be empty.".to_string());
        state.issue_draft.info_message = Some("old message".to_string());

        let outcome = handle_event(
            CrosstermEvent::Paste("Ship dashboard\nAdd tests".to_string()),
            &mut state,
        );

        assert!(!outcome.should_quit);
        assert!(outcome.submission.is_none());
        assert_eq!(state.issue_draft.prompt_text(), "Ship dashboard\nAdd tests");
        assert!(state.issue_draft.validation_error.is_none());
        assert!(state.issue_draft.info_message.is_none());
    }

    #[tokio::test]
    async fn submit_issue_sends_task_request() {
        let server = MockServer::start();
        let original_home = env::var_os("HOME");
        let temp = tempdir().expect("tempdir");
        env::set_var("HOME", temp.path());
        let auth_token_path = temp.path().join(".local/share/metis/auth-token");
        fs::create_dir_all(auth_token_path.parent().expect("auth token parent"))
            .expect("create auth token dir");
        fs::write(&auth_token_path, "token-123").expect("write auth token");
        let mock = server.mock(|when, then| {
            when.method(POST).path("/v1/issues").json_body(json!({
                "issue": {
                    "type": "task",
                    "description": "Draft release notes",
                    "creator": {
                        "username": " metis-user ",
                        "github_user_id": null,
                        "github_token": "token-123"
                    },
                    "progress": "",
                    "status": "open",
                    "assignee": "alice",
                    "dependencies": [],
                    "patches": []
                }
            }));
            then.status(200)
                .json_body_obj(&UpsertIssueResponse::new(issue_id("i-new")));
        });

        let client = MetisClient::new(server.base_url()).expect("failed to create client");

        let submission = IssueSubmission {
            prompt: "Draft release notes".to_string(),
            assignee: "alice".to_string(),
        };

        let created = submit_issue(&client, &submission, " metis-user ")
            .await
            .expect("submission failed");

        assert_eq!(created, issue_id("i-new"));
        mock.assert();
        match original_home {
            Some(value) => env::set_var("HOME", value),
            None => env::remove_var("HOME"),
        }
    }

    fn row_text(buffer: &Buffer, y: u16, width: u16) -> String {
        let mut row = String::new();
        for x in 0..width {
            row.push_str(buffer[(x, y)].symbol());
        }
        row.trim_end().to_string()
    }
}
