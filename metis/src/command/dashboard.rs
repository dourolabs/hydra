use std::{
    cmp::Ordering,
    collections::{BTreeSet, HashMap, HashSet},
    process::Command,
    time::Duration,
};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use crossterm::event::{
    Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};
use futures::StreamExt;
use metis_common::{
    api::v1::events::{EventsQuery, SseEventType},
    issues::{
        Issue, IssueDependency, IssueDependencyType, IssueRecord as ApiIssueRecord, IssueStatus,
        IssueType, JobSettings, SearchIssuesQuery, UpsertIssueRequest,
    },
    jobs::{JobRecord, SearchJobsQuery},
    patches::{GithubPr, PatchRecord},
    repositories::SearchRepositoriesQuery,
    task_status::{Status, TaskError, TaskStatusLog},
    users::Username,
    whoami::ActorIdentity,
    IssueId, PatchId, RepoName, RepositoryRecord, TaskId,
};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
    },
    DefaultTerminal, Frame,
};
use tui_textarea::TextArea;
use unicode_width::UnicodeWidthChar;

use crate::{
    client::{sse::SseEventStream, MetisClientInterface},
    command::{jobs, output::CommandContext},
};

pub mod panel;

use panel::{keybinding_line_from_labels, wrapped_content_len, Panel, PanelEvent, PanelState};

const JOB_REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const RECORD_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const SSE_POLL_FALLBACK_INTERVAL: Duration = Duration::from_secs(30);
const MAX_MESSAGE_WIDTH: usize = 90;
const USER_ISSUES_PANEL_CONTENT_HEIGHT: u16 = 5;
const USER_ISSUES_PANEL_HEIGHT: u16 = USER_ISSUES_PANEL_CONTENT_HEIGHT + 2;
const ISSUE_CREATOR_PANEL_INNER_HEIGHT: u16 = 10;
const ISSUE_CREATOR_PANEL_HEIGHT: u16 = ISSUE_CREATOR_PANEL_INNER_HEIGHT + 2;
const ISSUE_CREATOR_FOOTER_GAP: usize = 2;
#[derive(Copy, Clone, PartialEq, Default, Debug)]
enum PanelFocus {
    #[default]
    NewIssue,
    UserOwned,
    Running,
    Completed,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
enum IssueListFilter {
    #[default]
    All,
    CreatorOnly,
}

impl IssueListFilter {
    fn toggle(self) -> Self {
        match self {
            IssueListFilter::All => IssueListFilter::CreatorOnly,
            IssueListFilter::CreatorOnly => IssueListFilter::All,
        }
    }

    fn label(self) -> &'static str {
        match self {
            IssueListFilter::All => "All",
            IssueListFilter::CreatorOnly => "Creator-only",
        }
    }
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
    issue_type: IssueType,
    description: String,
    creator: Username,
    progress: String,
    status: IssueStatus,
    assignee: Option<String>,
    dependencies: Vec<IssueDependency>,
    patches: Vec<PatchId>,
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
    creator: Username,
    assignee: Option<String>,
    task: Option<TaskIndicator>,
    depth: usize,
    has_children: bool,
    collapsed: bool,
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
    repos: Vec<RepoName>,
    repo_index: usize,
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
            repos: Vec::new(),
            repo_index: 0,
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

    fn selected_repo(&self) -> Option<&RepoName> {
        self.repos.get(self.repo_index)
    }

    fn cycle_repo(&mut self, forward: bool) {
        if self.repos.is_empty() {
            return;
        }

        let total = self.repos.len();
        let next = if forward {
            (self.repo_index + 1) % total
        } else {
            self.repo_index.saturating_add(total - 1) % total
        };
        self.repo_index = next;
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
            && self.repos == other.repos
            && self.repo_index == other.repo_index
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

#[derive(Default, Clone, PartialEq)]
struct IssueSelectionState {
    index: usize,
}

#[derive(Default, Clone, PartialEq)]
struct IssueDetailsState {
    is_open: bool,
    issue_id: Option<IssueId>,
    scroll: ListScrollState,
    confirm_drop: bool,
}

#[derive(Clone)]
struct DashboardState {
    jobs: Vec<JobDetails>,
    issues: Vec<IssueRecord>,
    repositories: Vec<RepositoryRecord>,
    issue_lines: IssueLines,
    user_unowned_issue_lines: IssueLines,
    completed_issue_lines: CompletedIssueLines,
    collapsed_issue_ids: HashSet<IssueId>,
    known_completed_issue_ids: HashSet<IssueId>,
    // Keep per-panel selection indices so each list preserves context across refreshes.
    user_unowned_issue_selection: IssueSelectionState,
    running_issue_selection: IssueSelectionState,
    completed_issue_selection: IssueSelectionState,
    running_issue_panel: PanelState,
    user_unowned_issue_panel: PanelState,
    completed_issue_panel: PanelState,
    issue_creator_panel: PanelState,
    issue_draft_scroll: ListScrollState,
    issue_details: IssueDetailsState,
    issue_list_filter: IssueListFilter,
    jobs_error: Option<String>,
    records_error: Option<String>,
    username: Username,
    server_url: String,
    browser_command: Option<String>,
    issue_draft: IssueDraft,
    selected_panel: PanelFocus,
    last_frame_size: Option<Rect>,
}

impl Default for DashboardState {
    fn default() -> Self {
        let mut issue_creator_panel = PanelState::new();
        issue_creator_panel.set_scroll_keys_enabled(false);
        issue_creator_panel.register_keybinding(KeyCode::Char('a'), KeyModifiers::ALT, "Assignee");
        issue_creator_panel.register_keybinding(KeyCode::Char('r'), KeyModifiers::ALT, "Repo");
        issue_creator_panel.register_keybinding(KeyCode::Enter, KeyModifiers::ALT, "Submit");
        issue_creator_panel.register_keybinding(KeyCode::Tab, KeyModifiers::NONE, "Next panel");
        issue_creator_panel.register_keybinding(KeyCode::BackTab, KeyModifiers::NONE, "Prev panel");

        let mut running_issue_panel = PanelState::new();
        configure_issue_tree_panel_keybindings(&mut running_issue_panel);
        running_issue_panel.register_keybinding(
            KeyCode::Enter,
            KeyModifiers::NONE,
            "Open details/PR",
        );
        let mut user_unowned_issue_panel = PanelState::new();
        configure_status_panel_keybindings(&mut user_unowned_issue_panel);
        user_unowned_issue_panel.register_keybinding(KeyCode::Enter, KeyModifiers::NONE, "Open PR");
        let mut completed_issue_panel = PanelState::new();
        configure_issue_tree_panel_keybindings(&mut completed_issue_panel);
        completed_issue_panel.register_keybinding(
            KeyCode::Enter,
            KeyModifiers::NONE,
            "Open details/PR",
        );

        let mut state = Self {
            jobs: Vec::new(),
            issues: Vec::new(),
            repositories: Vec::new(),
            issue_lines: IssueLines::default(),
            user_unowned_issue_lines: IssueLines::default(),
            completed_issue_lines: CompletedIssueLines::default(),
            collapsed_issue_ids: HashSet::new(),
            known_completed_issue_ids: HashSet::new(),
            user_unowned_issue_selection: IssueSelectionState::default(),
            running_issue_selection: IssueSelectionState::default(),
            completed_issue_selection: IssueSelectionState::default(),
            running_issue_panel,
            user_unowned_issue_panel,
            completed_issue_panel,
            issue_creator_panel,
            issue_draft_scroll: ListScrollState::default(),
            issue_details: IssueDetailsState::default(),
            issue_list_filter: IssueListFilter::default(),
            jobs_error: None,
            records_error: None,
            username: Username::from(""),
            server_url: String::new(),
            browser_command: None,
            issue_draft: IssueDraft::default(),
            selected_panel: PanelFocus::default(),
            last_frame_size: None,
        };
        update_panel_focus(&mut state);
        state
    }
}

fn configure_status_panel_keybindings(panel: &mut PanelState) {
    panel.set_scroll_keys_enabled(false);
    panel.register_keybinding(KeyCode::Up, KeyModifiers::NONE, "Select");
    panel.register_keybinding(KeyCode::Down, KeyModifiers::NONE, "Select");
    panel.register_keybinding(KeyCode::Tab, KeyModifiers::NONE, "Next panel");
    panel.register_keybinding(KeyCode::BackTab, KeyModifiers::NONE, "Prev panel");
}

fn configure_issue_tree_panel_keybindings(panel: &mut PanelState) {
    configure_status_panel_keybindings(panel);
    panel.register_keybinding(KeyCode::Char(' '), KeyModifiers::NONE, "Expand/Collapse");
    panel.register_keybinding(KeyCode::Char('f'), KeyModifiers::ALT, "Filter");
}

struct IssueSubmission {
    prompt: String,
    assignee: String,
    repo_name: Option<RepoName>,
}

struct IssueStatusUpdate {
    issue_id: IssueId,
    status: IssueStatus,
}

struct EventOutcome {
    should_quit: bool,
    submission: Option<IssueSubmission>,
    open_issue_pr: Option<IssueId>,
    status_update: Option<IssueStatusUpdate>,
}

pub async fn run(
    client: &dyn MetisClientInterface,
    server_url: &str,
    browser_command: Option<&str>,
    _context: &CommandContext,
) -> Result<()> {
    let whoami = client
        .whoami()
        .await
        .context("failed to resolve authenticated actor")?;
    let username = match whoami.actor {
        ActorIdentity::User { username } => username,
        ActorIdentity::Task { task_id } => {
            bail!("dashboard requires a user token (got task {task_id})");
        }
        _ => bail!("dashboard requires a user token"),
    };
    let mut terminal = ratatui::init();
    let result =
        run_dashboard_loop(client, &mut terminal, username, server_url, browser_command).await;
    ratatui::restore();
    result
}

async fn run_dashboard_loop(
    client: &dyn MetisClientInterface,
    terminal: &mut DefaultTerminal,
    username: Username,
    server_url: &str,
    browser_command: Option<&str>,
) -> Result<()> {
    let mut state = DashboardState {
        username,
        server_url: server_url.to_string(),
        browser_command: browser_command.map(str::to_string),
        ..DashboardState::default()
    };
    update_panel_focus(&mut state);
    let mut needs_draw = true;

    if let Err(err) = refresh_jobs(client, &mut state).await {
        state.jobs_error = Some(format!("Failed to load jobs: {err}"));
    }

    if let Err(err) = refresh_records(client, &mut state).await {
        state.records_error = Some(format!("Failed to load records: {err}"));
    }

    // Try to connect to SSE for real-time updates.
    let mut sse_stream: Option<SseEventStream> = try_connect_sse(client).await;
    let sse_active = sse_stream.is_some();

    let mut events = EventStream::new();
    // Use longer polling intervals when SSE is active (heartbeat/consistency check).
    let mut jobs_tick = tokio::time::interval(if sse_active {
        SSE_POLL_FALLBACK_INTERVAL
    } else {
        JOB_REFRESH_INTERVAL
    });
    let mut records_tick = tokio::time::interval(if sse_active {
        SSE_POLL_FALLBACK_INTERVAL
    } else {
        RECORD_REFRESH_INTERVAL
    });

    loop {
        if needs_draw {
            state.last_frame_size = Some(terminal.size()?.into());
            clamp_issue_scrolls(&mut state);
            terminal.draw(|f| render(f, &mut state))?;
            needs_draw = false;
        }

        tokio::select! {
            // SSE event stream — only active when we have a connection.
            sse_event = async {
                match sse_stream.as_mut() {
                    Some(stream) => stream.next().await,
                    None => std::future::pending().await,
                }
            } => {
                match sse_event {
                    Some(Ok(event)) => {
                        match event.event_type {
                            SseEventType::JobCreated | SseEventType::JobUpdated => {
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
                            SseEventType::IssueCreated
                            | SseEventType::IssueUpdated
                            | SseEventType::IssueDeleted
                            | SseEventType::PatchCreated
                            | SseEventType::PatchUpdated
                            | SseEventType::PatchDeleted
                            | SseEventType::DocumentCreated
                            | SseEventType::DocumentUpdated
                            | SseEventType::DocumentDeleted => {
                                match refresh_records(client, &mut state).await {
                                    Ok(changed) => {
                                        state.records_error = None;
                                        needs_draw |= changed;
                                    }
                                    Err(err) => {
                                        state.records_error =
                                            Some(format!("Failed to refresh records: {err}"));
                                        needs_draw = true;
                                    }
                                }
                            }
                            SseEventType::Resync | SseEventType::Snapshot => {
                                // Full refresh on resync/snapshot.
                                if let Err(err) = refresh_jobs(client, &mut state).await {
                                    state.jobs_error = Some(format!("Failed to refresh jobs: {err}"));
                                }
                                if let Err(err) = refresh_records(client, &mut state).await {
                                    state.records_error =
                                        Some(format!("Failed to refresh records: {err}"));
                                }
                                needs_draw = true;
                            }
                            SseEventType::Heartbeat => {}
                        }
                    }
                    Some(Err(_)) | None => {
                        // SSE stream dropped — fall back to fast polling.
                        sse_stream = None;
                        jobs_tick = tokio::time::interval(JOB_REFRESH_INTERVAL);
                        records_tick = tokio::time::interval(RECORD_REFRESH_INTERVAL);
                    }
                }
            }
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
                        state.records_error = Some(format!("Failed to refresh records: {err}"));
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
                        if let Some(issue_id) = outcome.open_issue_pr {
                            if let Err(err) = open_issue_pr(client, &state, &issue_id).await {
                                state.records_error = Some(format!(
                                    "Failed to open pull request for {issue_id}: {err}"
                                ));
                            }
                        }
                        if let Some(update) = outcome.status_update {
                            let issue_id = update.issue_id.clone();
                            if let Err(err) = update_issue_status(client, &update).await {
                                state.records_error = Some(format!(
                                    "Failed to update issue {issue_id}: {err}"
                                ));
                            } else if let Err(err) = refresh_records(client, &mut state).await {
                                state.records_error =
                                    Some(format!("Failed to refresh records: {err}"));
                            }
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
                                            Some(format!("Failed to refresh records: {err}"));
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

/// Attempt to connect to the SSE events endpoint. Returns `None` if unavailable.
async fn try_connect_sse(client: &dyn MetisClientInterface) -> Option<SseEventStream> {
    client
        .subscribe_events(&EventsQuery::default(), None)
        .await
        .unwrap_or_default()
}

fn handle_event(event: Event, state: &mut DashboardState) -> EventOutcome {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                return EventOutcome {
                    should_quit: true,
                    submission: None,
                    open_issue_pr: None,
                    status_update: None,
                };
            }

            if state.issue_details.is_open {
                let status_update = handle_issue_details_key(key, state);
                return EventOutcome {
                    should_quit: false,
                    submission: None,
                    open_issue_pr: None,
                    status_update,
                };
            }

            if is_issue_filter_toggle_key(key) {
                toggle_issue_list_filter(state);
                return EventOutcome {
                    should_quit: false,
                    submission: None,
                    open_issue_pr: None,
                    status_update: None,
                };
            }

            if is_panel_focus_key(key) {
                handle_panel_focus_key(key, state);
                return EventOutcome {
                    should_quit: false,
                    submission: None,
                    open_issue_pr: None,
                    status_update: None,
                };
            }

            if is_issue_tree_space_key(key, state) {
                toggle_selected_issue_children(state);
                return EventOutcome {
                    should_quit: false,
                    submission: None,
                    open_issue_pr: None,
                    status_update: None,
                };
            }

            if is_issue_pr_open_key(key, state) {
                return EventOutcome {
                    should_quit: false,
                    submission: None,
                    open_issue_pr: selected_issue_id(state, PanelFocus::UserOwned),
                    status_update: None,
                };
            }

            if let Some(issue_id) = merge_request_pr_open_id(key, state) {
                return EventOutcome {
                    should_quit: false,
                    submission: None,
                    open_issue_pr: Some(issue_id),
                    status_update: None,
                };
            }

            if let Some(issue_id) = issue_details_open_id(key, state) {
                open_issue_details(state, issue_id);
                return EventOutcome {
                    should_quit: false,
                    submission: None,
                    open_issue_pr: None,
                    status_update: None,
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
                open_issue_pr: None,
                status_update: None,
            }
        }
        Event::Paste(text) => {
            if state.issue_details.is_open {
                return EventOutcome {
                    should_quit: false,
                    submission: None,
                    open_issue_pr: None,
                    status_update: None,
                };
            }
            if state.issue_draft.is_submitting {
                return EventOutcome {
                    should_quit: false,
                    submission: None,
                    open_issue_pr: None,
                    status_update: None,
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
                open_issue_pr: None,
                status_update: None,
            }
        }
        Event::Resize(width, height) => {
            state.last_frame_size = Some(Rect::new(0, 0, width, height));
            clamp_issue_scrolls(state);
            EventOutcome {
                should_quit: false,
                submission: None,
                open_issue_pr: None,
                status_update: None,
            }
        }
        Event::Mouse(mouse) => {
            if state.issue_details.is_open {
                return EventOutcome {
                    should_quit: false,
                    submission: None,
                    open_issue_pr: None,
                    status_update: None,
                };
            }
            handle_mouse_scroll(mouse, state);
            handle_mouse_click(mouse, state);
            EventOutcome {
                should_quit: false,
                submission: None,
                open_issue_pr: None,
                status_update: None,
            }
        }
        _ => EventOutcome {
            should_quit: false,
            submission: None,
            open_issue_pr: None,
            status_update: None,
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

fn is_simple_char_key(key: KeyEvent, target: char) -> bool {
    matches!(key.code, KeyCode::Char(ch) if ch.eq_ignore_ascii_case(&target))
        && !key.modifiers.contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::ALT)
        && !key.modifiers.contains(KeyModifiers::META)
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

fn is_issue_filter_toggle_key(key: KeyEvent) -> bool {
    is_alt_char_key(key, 'f')
}

fn is_issue_tree_space_key(key: KeyEvent, state: &DashboardState) -> bool {
    if !key.modifiers.is_empty() || state.issue_creator_panel.focused() {
        return false;
    }

    matches!(key.code, KeyCode::Char(' '))
        && matches!(
            state.selected_panel,
            PanelFocus::Running | PanelFocus::Completed
        )
}

fn is_issue_pr_open_key(key: KeyEvent, state: &DashboardState) -> bool {
    if key.code != KeyCode::Enter || !key.modifiers.is_empty() {
        return false;
    }

    matches!(state.selected_panel, PanelFocus::UserOwned)
        && selected_issue_id(state, PanelFocus::UserOwned).is_some()
}

fn merge_request_pr_open_id(key: KeyEvent, state: &DashboardState) -> Option<IssueId> {
    if key.code != KeyCode::Enter || !key.modifiers.is_empty() {
        return None;
    }

    match state.selected_panel {
        PanelFocus::Running | PanelFocus::Completed => {}
        _ => return None,
    }

    let issue_id = selected_issue_id(state, state.selected_panel)?;
    let issue = state.issues.iter().find(|issue| issue.id == issue_id)?;
    if issue.issue_type == IssueType::MergeRequest {
        Some(issue.id.clone())
    } else {
        None
    }
}

fn issue_details_open_id(key: KeyEvent, state: &DashboardState) -> Option<IssueId> {
    if key.code != KeyCode::Enter || !key.modifiers.is_empty() {
        return None;
    }

    match state.selected_panel {
        PanelFocus::Running | PanelFocus::Completed => {}
        _ => return None,
    }

    let issue_id = selected_issue_id(state, state.selected_panel)?;
    let issue = state.issues.iter().find(|issue| issue.id == issue_id)?;
    if issue.issue_type == IssueType::MergeRequest {
        None
    } else {
        Some(issue.id.clone())
    }
}

fn selection_key_delta(key: KeyEvent) -> Option<i32> {
    if !key.modifiers.is_empty() {
        return None;
    }

    match key.code {
        KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('K') => Some(-1),
        KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('J') => Some(1),
        _ => None,
    }
}

fn issue_details_issue(state: &DashboardState) -> Option<&IssueRecord> {
    let issue_id = state.issue_details.issue_id.as_ref()?;
    state.issues.iter().find(|issue| &issue.id == issue_id)
}

fn is_issue_drop_key(key: KeyEvent, state: &DashboardState) -> bool {
    if !is_alt_char_key(key, 'd') {
        return false;
    }

    let Some(issue) = issue_details_issue(state) else {
        return false;
    };

    issue.status != IssueStatus::Dropped
}

fn handle_issue_drop_confirmation_key(
    key: KeyEvent,
    state: &mut DashboardState,
) -> Option<IssueStatusUpdate> {
    if key.modifiers.is_empty() && key.code == KeyCode::Esc {
        state.issue_details.confirm_drop = false;
        return None;
    }

    if is_simple_char_key(key, 'n') {
        state.issue_details.confirm_drop = false;
        return None;
    }

    if key.code == KeyCode::Enter || is_simple_char_key(key, 'y') {
        state.issue_details.confirm_drop = false;
        return state
            .issue_details
            .issue_id
            .clone()
            .map(|issue_id| IssueStatusUpdate {
                issue_id,
                status: IssueStatus::Dropped,
            });
    }

    None
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

fn toggle_issue_list_filter(state: &mut DashboardState) {
    state.issue_list_filter = state.issue_list_filter.toggle();
    update_views(state);
    scroll_selected_issue_into_view(state);
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

    if is_alt_char_key(key, 'r') {
        state.issue_draft.cycle_repo(true);
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

fn handle_issue_details_key(
    key: KeyEvent,
    state: &mut DashboardState,
) -> Option<IssueStatusUpdate> {
    if state.issue_details.confirm_drop {
        return handle_issue_drop_confirmation_key(key, state);
    }

    if is_issue_drop_key(key, state) {
        state.issue_details.confirm_drop = true;
        return None;
    }

    if key.modifiers.is_empty() && matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
        close_issue_details(state);
        return None;
    }

    let delta = selection_key_delta(key)?;
    scroll_issue_details(state, delta);
    None
}

fn handle_status_panel_key(key: KeyEvent, state: &mut DashboardState) -> bool {
    let Some(delta) = selection_key_delta(key) else {
        return false;
    };

    let moved = match state.selected_panel {
        PanelFocus::UserOwned => move_issue_selection(
            &mut state.user_unowned_issue_selection,
            state.user_unowned_issue_lines.rows.len(),
            delta,
        ),
        PanelFocus::Running => move_issue_selection(
            &mut state.running_issue_selection,
            state.issue_lines.rows.len(),
            delta,
        ),
        PanelFocus::Completed => {
            let rows = completed_issue_rows(&state.completed_issue_lines);
            move_issue_selection(&mut state.completed_issue_selection, rows.len(), delta)
        }
        PanelFocus::NewIssue => false,
    };

    if moved {
        scroll_selected_issue_into_view(state);
    }

    moved
}

// Toggle handler for issue tree expansion; invoke this when a keybinding (e.g. spacebar)
// should expand or collapse the children of the currently selected issue.
fn toggle_selected_issue_children(state: &mut DashboardState) -> bool {
    let selected_panel = state.selected_panel;
    let Some(issue_id) = selected_issue_id(state, selected_panel) else {
        return false;
    };

    let nodes = build_issue_nodes(&state.issues, &state.jobs);
    let Some(node) = nodes.get(&issue_id) else {
        return false;
    };
    if node.children.is_empty() {
        return false;
    }

    if state.collapsed_issue_ids.contains(&issue_id) {
        state.collapsed_issue_ids.remove(&issue_id);
    } else {
        state.collapsed_issue_ids.insert(issue_id.clone());
    }

    update_views(state);
    if restore_issue_selection(state, selected_panel, &issue_id) {
        scroll_selected_issue_into_view(state);
    }
    true
}

fn selected_issue_id(state: &DashboardState, panel: PanelFocus) -> Option<IssueId> {
    match panel {
        PanelFocus::Running => {
            issue_id_for_selection(&state.issue_lines.rows, &state.running_issue_selection)
        }
        PanelFocus::Completed => {
            let rows = completed_issue_rows(&state.completed_issue_lines);
            issue_id_for_selection(&rows, &state.completed_issue_selection)
        }
        PanelFocus::UserOwned => issue_id_for_selection(
            &state.user_unowned_issue_lines.rows,
            &state.user_unowned_issue_selection,
        ),
        PanelFocus::NewIssue => None,
    }
}

fn issue_id_for_selection(
    issue_lines: &[IssueLine],
    selection: &IssueSelectionState,
) -> Option<IssueId> {
    let index = selection_index(selection, issue_lines.len())?;
    issue_lines.get(index)?.id.parse::<IssueId>().ok()
}

fn restore_issue_selection(
    state: &mut DashboardState,
    panel: PanelFocus,
    issue_id: &IssueId,
) -> bool {
    match panel {
        PanelFocus::Running => set_selection_for_issue(
            &mut state.running_issue_selection,
            &state.issue_lines.rows,
            issue_id,
        ),
        PanelFocus::Completed => {
            let rows = completed_issue_rows(&state.completed_issue_lines);
            set_selection_for_issue(&mut state.completed_issue_selection, &rows, issue_id)
        }
        PanelFocus::UserOwned => false,
        PanelFocus::NewIssue => false,
    }
}

fn set_selection_for_issue(
    selection: &mut IssueSelectionState,
    issue_lines: &[IssueLine],
    issue_id: &IssueId,
) -> bool {
    if let Some(index) = issue_lines
        .iter()
        .position(|line| line.id == issue_id.as_ref())
    {
        selection.index = index;
        true
    } else {
        false
    }
}

async fn open_issue_pr(
    client: &dyn MetisClientInterface,
    state: &DashboardState,
    issue_id: &IssueId,
) -> Result<()> {
    let browser_command = state
        .browser_command
        .as_deref()
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .context("browser command is not configured")?;
    let issue = state
        .issues
        .iter()
        .find(|issue| issue.id == *issue_id)
        .context("issue not found")?;
    let mut last_error = None;

    for patch_id in &issue.patches {
        match client.get_patch(patch_id).await {
            Ok(patch) => {
                if let Some(url) = patch_pr_url(&patch) {
                    return open_browser(browser_command, &url);
                }
            }
            Err(err) => {
                last_error = Some(err);
            }
        }
    }

    if let Some(err) = last_error {
        Err(err).context("failed to fetch patch for issue")
    } else {
        anyhow::bail!("no pull request found for issue")
    }
}

fn patch_pr_url(patch: &PatchRecord) -> Option<String> {
    patch.patch.github.as_ref().map(github_pr_url)
}

fn github_pr_url(github: &GithubPr) -> String {
    if let Some(url) = &github.url {
        let trimmed = url.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    format!(
        "https://github.com/{}/{}/pull/{}",
        github.owner, github.repo, github.number
    )
}

fn parse_browser_command(browser_command: &str) -> Result<(String, Vec<String>)> {
    let trimmed = browser_command.trim();
    if trimmed.is_empty() {
        anyhow::bail!("browser command is empty");
    }

    let parts = shlex::split(trimmed).context("browser command has invalid quoting")?;
    let (command, args) = parts.split_first().context("browser command is empty")?;
    Ok((command.to_string(), args.to_vec()))
}

fn open_browser(browser_command: &str, url: &str) -> Result<()> {
    let (command, args) = parse_browser_command(browser_command)?;
    let mut cmd = Command::new(command);
    cmd.args(args);
    cmd.arg(url);
    cmd.spawn().context("failed to launch browser")?;
    Ok(())
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
        repo_name: state.issue_draft.selected_repo().cloned(),
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
    creator: &Username,
) -> Result<IssueId> {
    let assignee = submission.assignee.trim();
    let assignee = if assignee.is_empty() {
        None
    } else {
        Some(assignee.to_string())
    };

    let request = UpsertIssueRequest::new(
        Issue::new(
            IssueType::Task,
            submission.prompt.trim().to_string(),
            creator.clone(),
            String::new(),
            IssueStatus::Open,
            assignee,
            submission.repo_name.as_ref().map(|repo_name| {
                let mut settings = JobSettings::default();
                settings.repo_name = Some(repo_name.clone());
                settings
            }),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            false,
        ),
        None,
    );

    let response = client
        .create_issue(&request)
        .await
        .context("failed to create issue")?;
    Ok(response.issue_id)
}

async fn update_issue_status(
    client: &dyn MetisClientInterface,
    update: &IssueStatusUpdate,
) -> Result<IssueId> {
    let current = client
        .get_issue(&update.issue_id)
        .await
        .with_context(|| format!("failed to fetch issue '{}'", update.issue_id))?;
    let issue = current.issue;
    let updated_issue = Issue::new(
        issue.issue_type,
        issue.description,
        issue.creator,
        issue.progress,
        update.status,
        issue.assignee,
        Some(issue.job_settings),
        issue.todo_list,
        issue.dependencies,
        issue.patches,
        issue.deleted,
    );
    let response = client
        .update_issue(
            &update.issue_id,
            &UpsertIssueRequest::new(updated_issue, None),
        )
        .await
        .with_context(|| format!("failed to update issue '{}'", update.issue_id))?;
    Ok(response.issue_id)
}

fn render(frame: &mut Frame, state: &mut DashboardState) {
    let layout = dashboard_layout(frame.area());
    render_dashboard_header(
        frame,
        layout.header,
        state.username.as_str(),
        &state.server_url,
        state.issue_list_filter,
    );
    render_issue_creator(frame, layout.issue_creator, state);
    render_issue_sections(frame, layout.issue_sections, state);
    if state.issue_details.is_open {
        render_issue_details(frame, state);
        if state.issue_details.confirm_drop {
            render_issue_drop_confirmation(frame, state);
        }
    }
}

fn render_dashboard_header(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    username: &str,
    server_url: &str,
    issue_list_filter: IssueListFilter,
) {
    let title = dashboard_title(username, server_url);
    let hint = format!(
        "Tab/Shift+Tab to change panels, Alt+F to filter issues ({}), Ctrl+C to exit.",
        issue_list_filter.label()
    );
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

fn dashboard_title(username: &str, server_url: &str) -> String {
    let trimmed = username.trim();
    let server_url = format_server_url(server_url);
    if trimmed.is_empty() {
        "Metis Dashboard".to_string()
    } else if let Some(server_url) = server_url {
        format!("Metis Dashboard — {trimmed} @ {server_url}")
    } else {
        format!("Metis Dashboard — {trimmed}")
    }
}

fn format_server_url(server_url: &str) -> Option<String> {
    let trimmed = server_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
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
        let focused = state.selected_panel == PanelFocus::UserOwned;
        let selection = selection_index(
            &state.user_unowned_issue_selection,
            state.user_unowned_issue_lines.rows.len(),
        );
        let lines = issue_line_lines(
            &state.user_unowned_issue_lines.rows,
            "No open issues assigned to you",
            false,
            state.username.as_str(),
            selection,
            focused,
        );
        let panel = Panel::new(Line::from(title), lines);
        frame.render_stateful_widget(panel, rect, &mut state.user_unowned_issue_panel);
    }

    let running_focused = state.selected_panel == PanelFocus::Running;
    let running_selection =
        selection_index(&state.running_issue_selection, state.issue_lines.rows.len());
    let running_title = issue_list_title("Running issues", &state.issue_lines);
    let running_lines = issue_line_lines(
        &state.issue_lines.rows,
        "No issues found",
        true,
        state.username.as_str(),
        running_selection,
        running_focused,
    );
    let running_panel = Panel::new(Line::from(running_title), running_lines);
    frame.render_stateful_widget(
        running_panel,
        panels.running,
        &mut state.running_issue_panel,
    );

    let completed_title = completed_issue_list_title(&state.completed_issue_lines);
    let completed_rows = completed_issue_rows(&state.completed_issue_lines);
    let completed_focused = state.selected_panel == PanelFocus::Completed;
    let completed_selection =
        selection_index(&state.completed_issue_selection, completed_rows.len());
    let completed_lines = issue_line_lines(
        &completed_rows,
        "No completed issues",
        true,
        state.username.as_str(),
        completed_selection,
        completed_focused,
    );
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
    let prompt_width = sections.prompt_input.width as usize;
    let prompt_lines = issue_draft_prompt_lines(&draft.prompt, prompt_width);
    let scroll_offset = state.issue_draft_scroll.offset.min(u16::MAX as usize) as u16;
    let prompt = Paragraph::new(prompt_lines)
        .alignment(draft.prompt.alignment())
        .scroll((scroll_offset, 0));
    frame.render_widget(prompt, sections.prompt_input);
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
    let repo_label = draft
        .selected_repo()
        .map(ToString::to_string)
        .unwrap_or_else(|| "-".to_string());
    let repo_line = Line::from(vec![
        Span::styled("Repo: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(repo_label, Style::default().fg(Color::Yellow)),
    ]);
    let footer_width = sections.footer.width as usize;
    let assignee_width = assignee_line.width().min(footer_width);
    let footer_gap = ISSUE_CREATOR_FOOTER_GAP.min(footer_width.saturating_sub(assignee_width));
    let repo_width = repo_line
        .width()
        .min(footer_width.saturating_sub(assignee_width + footer_gap));

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
        .constraints([
            Constraint::Length(assignee_width as u16),
            Constraint::Length(footer_gap as u16),
            Constraint::Length(repo_width as u16),
            Constraint::Min(0),
        ])
        .split(sections.footer);
    frame.render_widget(Paragraph::new(assignee_line), footer_columns[0]);
    frame.render_widget(Paragraph::new(repo_line), footer_columns[2]);
    frame.render_widget(
        Paragraph::new(footer).alignment(Alignment::Right),
        footer_columns[3],
    );
}

fn render_issue_details(frame: &mut Frame, state: &mut DashboardState) {
    let Some(view) = issue_details_view(state) else {
        return;
    };

    frame.render_widget(Clear, view.layout.area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::White))
        .title(view.title);
    frame.render_widget(block, view.layout.area);

    let content = Paragraph::new(view.lines)
        .scroll((view.scroll_offset, 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(content, view.layout.content);

    if view.content_len > view.view_height {
        render_panel_scrollbar(frame, view.layout.content, view.scrollbar_state);
    }

    let footer = Paragraph::new(view.keybinding_line).wrap(Wrap { trim: true });
    frame.render_widget(footer, view.layout.footer);
}

fn render_issue_drop_confirmation(frame: &mut Frame, state: &DashboardState) {
    if !state.issue_details.confirm_drop {
        return;
    }

    let Some(issue_id) = state.issue_details.issue_id.as_ref() else {
        return;
    };

    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }

    let width = (area.width.saturating_mul(55) / 100)
        .max(30)
        .min(area.width);
    let height = (area.height.saturating_mul(25) / 100)
        .max(7)
        .min(area.height);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    let popup = Rect::new(x, y, width, height);
    let inner = popup.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);

    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::White))
        .title("Confirm drop");
    frame.render_widget(block, popup);

    let content = vec![
        Line::from(Span::styled(
            format!("Drop issue {issue_id}?"),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from("This will set the status to dropped."),
    ];
    frame.render_widget(
        Paragraph::new(content).wrap(Wrap { trim: true }),
        sections[0],
    );

    let keybindings =
        keybinding_line_from_labels(&[("y", "Drop"), ("n", "Cancel"), ("Esc", "Cancel")], true);
    frame.render_widget(
        Paragraph::new(keybindings).wrap(Wrap { trim: true }),
        sections[1],
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
        if let Some(descendants) = root
            .id
            .parse::<IssueId>()
            .ok()
            .and_then(|id| completed_issue_descendants(completed_issue_lines, &id))
        {
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
                state.username.as_str(),
                None,
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
            let lines = issue_line_lines(
                &state.issue_lines.rows,
                "No issues found",
                true,
                state.username.as_str(),
                None,
                false,
            );
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
            let lines = issue_line_lines(
                &rows,
                "No completed issues",
                true,
                state.username.as_str(),
                None,
                false,
            );
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
            state.username.as_str(),
            None,
            false,
        );
        let (content_len, view_height) = panel_scroll_metrics(rect, &lines);
        state
            .user_unowned_issue_panel
            .sync_scroll(content_len, view_height);
    } else {
        state.user_unowned_issue_panel.sync_scroll(0, 0);
    }

    let running_lines = issue_line_lines(
        &state.issue_lines.rows,
        "No issues found",
        true,
        state.username.as_str(),
        None,
        false,
    );
    let (running_len, running_view_height) = panel_scroll_metrics(panels.running, &running_lines);
    state
        .running_issue_panel
        .sync_scroll(running_len, running_view_height);

    let completed_rows = completed_issue_rows(&state.completed_issue_lines);
    let completed_lines = issue_line_lines(
        &completed_rows,
        "No completed issues",
        true,
        state.username.as_str(),
        None,
        false,
    );
    let (completed_len, completed_view_height) =
        panel_scroll_metrics(panels.completed, &completed_lines);
    state
        .completed_issue_panel
        .sync_scroll(completed_len, completed_view_height);

    let creator_layout = issue_creator_layout(layout.issue_creator);
    let prompt_view_height = creator_layout.prompt_input.height as usize;
    let prompt_width = creator_layout.prompt_input.width as usize;
    let (prompt_lines, cursor_row) =
        issue_draft_prompt_metrics(&state.issue_draft.prompt, prompt_width);
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

    if state.issue_details.is_open {
        issue_details_view(state);
    }
}

fn scroll_selected_issue_into_view(state: &mut DashboardState) -> bool {
    let size = match state.last_frame_size {
        Some(size) => size,
        None => return false,
    };
    let layout = dashboard_layout(size);
    let panels = issue_panel_layout(layout.issue_sections);

    match state.selected_panel {
        PanelFocus::UserOwned => {
            let Some(rect) = panels.user_owned else {
                return false;
            };
            let selection = selection_index(
                &state.user_unowned_issue_selection,
                state.user_unowned_issue_lines.rows.len(),
            );
            let lines = issue_line_lines(
                &state.user_unowned_issue_lines.rows,
                "No open issues assigned to you",
                false,
                state.username.as_str(),
                selection,
                true,
            );
            scroll_issue_panel_to_selection(
                &mut state.user_unowned_issue_panel,
                rect,
                &lines,
                selection,
            )
        }
        PanelFocus::Running => {
            let selection =
                selection_index(&state.running_issue_selection, state.issue_lines.rows.len());
            let lines = issue_line_lines(
                &state.issue_lines.rows,
                "No issues found",
                true,
                state.username.as_str(),
                selection,
                true,
            );
            scroll_issue_panel_to_selection(
                &mut state.running_issue_panel,
                panels.running,
                &lines,
                selection,
            )
        }
        PanelFocus::Completed => {
            let rows = completed_issue_rows(&state.completed_issue_lines);
            let selection = selection_index(&state.completed_issue_selection, rows.len());
            let lines = issue_line_lines(
                &rows,
                "No completed issues",
                true,
                state.username.as_str(),
                selection,
                true,
            );
            scroll_issue_panel_to_selection(
                &mut state.completed_issue_panel,
                panels.completed,
                &lines,
                selection,
            )
        }
        PanelFocus::NewIssue => false,
    }
}

fn completed_issue_descendants<'a>(
    completed_issue_lines: &'a CompletedIssueLines,
    root_id: &IssueId,
) -> Option<&'a Vec<IssueLine>> {
    completed_issue_lines.descendants.get(root_id)
}

fn issue_line_lines(
    issue_lines: &[IssueLine],
    empty_message: &str,
    show_hierarchy: bool,
    current_username: &str,
    selected: Option<usize>,
    focused: bool,
) -> Vec<Line<'static>> {
    if issue_lines.is_empty() {
        return vec![Line::from(Span::styled(
            empty_message.to_string(),
            Style::default().fg(Color::DarkGray),
        ))];
    }

    issue_lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let mut spans = Vec::new();
            if show_hierarchy {
                spans.push(Span::raw(issue_prefix(
                    line.depth,
                    line.has_children,
                    line.collapsed,
                )));
                spans.push(Span::raw(" "));
            }
            let (issue_status_label, issue_status_style) =
                issue_status_display(line.status, &line.readiness);
            spans.push(Span::styled(
                format!("[{issue_status_label}]"),
                issue_status_style,
            ));
            if should_render_creator(current_username, &line.creator) {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    format!("@{}", line.creator.as_str()),
                    issue_creator_style(),
                ));
            }

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

            let mut line = Line::from(spans);
            if focused && selected == Some(index) {
                line = highlight_line(line);
            }
            line
        })
        .collect()
}

fn highlight_line(mut line: Line<'static>) -> Line<'static> {
    let selection_style = Style::default().add_modifier(Modifier::REVERSED);
    line.spans = line
        .spans
        .into_iter()
        .map(|span| Span::styled(span.content, span.style.add_modifier(Modifier::REVERSED)))
        .collect();
    line.style = line.style.patch(selection_style);
    line
}

fn should_render_creator(current_username: &str, creator: &Username) -> bool {
    let current = current_username.trim();
    if current.is_empty() {
        return false;
    }
    creator.as_str().trim() != current
}

fn issue_creator_style() -> Style {
    Style::default()
        .fg(Color::Magenta)
        .add_modifier(Modifier::BOLD)
}

fn move_issue_selection(
    selection: &mut IssueSelectionState,
    total_items: usize,
    delta: i32,
) -> bool {
    if total_items == 0 {
        return false;
    }

    let current = selection.index;
    let next = if delta < 0 {
        current.saturating_sub(delta.unsigned_abs() as usize)
    } else {
        current.saturating_add(delta as usize)
    };
    let clamped = next.min(total_items.saturating_sub(1));
    if clamped == current {
        return false;
    }
    selection.index = clamped;
    true
}

fn selection_index(selection: &IssueSelectionState, total_items: usize) -> Option<usize> {
    if total_items == 0 {
        None
    } else {
        Some(selection.index.min(total_items.saturating_sub(1)))
    }
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

fn scroll_issue_panel_to_selection(
    panel: &mut PanelState,
    area: Rect,
    lines: &[Line<'static>],
    selection: Option<usize>,
) -> bool {
    let selection = match selection {
        Some(selection) => selection,
        None => return false,
    };

    if lines.is_empty() {
        return false;
    }

    let content_area = panel_content_area(area);
    let view_height = content_area.height as usize;
    let width = content_area.width as usize;
    if view_height == 0 || width == 0 || selection >= lines.len() {
        return false;
    }

    let content_len = wrapped_content_len(lines, content_area.width);
    let mut cursor_row: usize = 0;
    for line in lines.iter().take(selection) {
        cursor_row = cursor_row.saturating_add(wrapped_issue_line_len(line, width));
    }
    let selected_height = wrapped_issue_line_len(&lines[selection], width);
    let end_row = cursor_row.saturating_add(selected_height.saturating_sub(1));

    let current_top = panel.scroll_offset();
    let next_top = if cursor_row < current_top {
        cursor_row
    } else if current_top.saturating_add(view_height) <= end_row {
        end_row.saturating_add(1).saturating_sub(view_height)
    } else {
        current_top
    };

    let max_offset = max_scroll_offset(content_len, view_height);
    let next_top = next_top.min(max_offset);
    if next_top == current_top {
        return false;
    }

    panel.apply_scroll_delta(
        next_top as i32 - current_top as i32,
        content_len,
        view_height,
    )
}

fn wrapped_issue_line_len(line: &Line<'_>, width: usize) -> usize {
    let line_width = line.width();
    let wrapped = line_width.saturating_add(width.saturating_sub(1)) / width;
    wrapped.max(1)
}

#[derive(Clone, Debug)]
struct WrappedSegment {
    text: String,
    start_char: usize,
    end_char: usize,
}

fn wrap_line_segments(line: &str, width: usize) -> Vec<WrappedSegment> {
    if width == 0 {
        return Vec::new();
    }

    if line.is_empty() {
        return vec![WrappedSegment {
            text: String::new(),
            start_char: 0,
            end_char: 0,
        }];
    }

    let mut segments = Vec::new();
    let mut current = String::new();
    let mut current_width = 0;
    let mut segment_start = 0;
    let mut char_index = 0;

    for ch in line.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if current_width + ch_width > width && current_width > 0 {
            segments.push(WrappedSegment {
                text: current,
                start_char: segment_start,
                end_char: char_index,
            });
            current = String::new();
            current_width = 0;
            segment_start = char_index;
        }

        current.push(ch);
        current_width = current_width.saturating_add(ch_width);
        char_index += 1;

        if current_width >= width {
            segments.push(WrappedSegment {
                text: current,
                start_char: segment_start,
                end_char: char_index,
            });
            current = String::new();
            current_width = 0;
            segment_start = char_index;
        }
    }

    if !current.is_empty() || segments.is_empty() {
        segments.push(WrappedSegment {
            text: current,
            start_char: segment_start,
            end_char: char_index,
        });
    }

    segments
}

fn issue_draft_prompt_metrics(prompt: &TextArea<'_>, width: usize) -> (usize, usize) {
    if width == 0 {
        return (0, 0);
    }

    if prompt.is_empty() && !prompt.placeholder_text().is_empty() {
        let mut total: usize = 0;
        for line in prompt.placeholder_text().lines() {
            total = total.saturating_add(wrap_line_segments(line, width).len().max(1));
        }
        return (total.max(1), 0);
    }

    let cursor = prompt.cursor();
    let mut total: usize = 0;
    let mut cursor_row = 0;

    for (row_index, line) in prompt.lines().iter().enumerate() {
        let segments = wrap_line_segments(line, width);
        let segment_count = segments.len().max(1);

        if row_index == cursor.0 {
            let line_char_len = line.chars().count();
            let mut cursor_segment = segment_count.saturating_sub(1);
            for (segment_index, segment) in segments.iter().enumerate() {
                if cursor.1 < segment.end_char
                    || (cursor.1 == line_char_len && segment.end_char == line_char_len)
                {
                    cursor_segment = segment_index;
                    break;
                }
            }
            cursor_row = total.saturating_add(cursor_segment);
        }

        total = total.saturating_add(segment_count);
    }

    (total.max(1), cursor_row)
}

fn issue_draft_prompt_lines(prompt: &TextArea<'_>, width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return Vec::new();
    }

    if prompt.is_empty() && !prompt.placeholder_text().is_empty() {
        let placeholder_style = prompt.placeholder_style().unwrap_or_default();
        let mut lines = Vec::new();
        for line in prompt.placeholder_text().lines() {
            for segment in wrap_line_segments(line, width) {
                lines.push(Line::styled(segment.text, placeholder_style));
            }
        }
        return lines;
    }

    let cursor = prompt.cursor();
    let cursor_line_style = prompt.cursor_line_style();
    let cursor_style = prompt.cursor_style();
    let base_style = prompt.style();
    let mut lines = Vec::new();

    for (row_index, line) in prompt.lines().iter().enumerate() {
        let line_style = if row_index == cursor.0 {
            base_style.patch(cursor_line_style)
        } else {
            base_style
        };
        let segments = wrap_line_segments(line, width);
        let line_char_len = line.chars().count();
        let cursor_segment = if row_index == cursor.0 {
            segments
                .iter()
                .enumerate()
                .find_map(|(index, segment)| {
                    if cursor.1 < segment.end_char
                        || (cursor.1 == line_char_len && segment.end_char == line_char_len)
                    {
                        Some(index)
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| segments.len().saturating_sub(1))
        } else {
            usize::MAX
        };

        for (segment_index, segment) in segments.into_iter().enumerate() {
            if row_index == cursor.0 && segment_index == cursor_segment {
                let cursor_offset = cursor.1.saturating_sub(segment.start_char);
                let cursor_at_end = cursor.1 == line_char_len && segment.end_char == line_char_len;
                lines.push(prompt_segment_with_cursor(
                    segment.text,
                    cursor_offset,
                    cursor_at_end,
                    line_style,
                    line_style.patch(cursor_style),
                ));
            } else {
                lines.push(Line::styled(segment.text, line_style));
            }
        }
    }

    lines
}

fn prompt_segment_with_cursor(
    segment: String,
    cursor_offset: usize,
    cursor_at_end: bool,
    line_style: Style,
    cursor_style: Style,
) -> Line<'static> {
    let mut spans = Vec::new();
    if segment.is_empty() && cursor_at_end {
        spans.push(Span::styled(" ", cursor_style));
        return Line::from(spans);
    }

    let mut before = String::new();
    let mut after = String::new();
    let mut cursor_char = None;

    for (index, ch) in segment.chars().enumerate() {
        if index < cursor_offset {
            before.push(ch);
        } else if index == cursor_offset {
            cursor_char = Some(ch);
        } else {
            after.push(ch);
        }
    }

    if !before.is_empty() {
        spans.push(Span::styled(before, line_style));
    }
    if let Some(ch) = cursor_char {
        spans.push(Span::styled(ch.to_string(), cursor_style));
    } else if cursor_at_end {
        spans.push(Span::styled(" ", cursor_style));
    }
    if !after.is_empty() {
        spans.push(Span::styled(after, line_style));
    }

    if spans.is_empty() {
        spans.push(Span::styled(" ", cursor_style));
    }

    Line::from(spans)
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

struct IssueDetailsLayout {
    area: Rect,
    content: Rect,
    footer: Rect,
}

struct IssueDetailsView<'a> {
    layout: IssueDetailsLayout,
    lines: Vec<Line<'static>>,
    title: Line<'a>,
    keybinding_line: Line<'static>,
    content_len: usize,
    view_height: usize,
    scroll_offset: u16,
    scrollbar_state: ScrollbarState,
}

fn issue_details_view(state: &mut DashboardState) -> Option<IssueDetailsView<'static>> {
    let issue_id = state.issue_details.issue_id.clone()?;
    let issue = state.issues.iter().find(|issue| issue.id == issue_id)?;
    let size = state.last_frame_size?;
    let layout = issue_details_layout(size);
    if layout.area.width == 0 || layout.area.height == 0 {
        return None;
    }

    let lines = issue_detail_lines(issue, state.username.as_str());
    let view_height = layout.content.height as usize;
    let content_len = wrapped_content_len(&lines, layout.content.width);
    let max_offset = max_scroll_offset(content_len, view_height);
    let scroll_offset = state.issue_details.scroll.offset.min(max_offset);
    state.issue_details.scroll.offset = scroll_offset;
    state.issue_details.scroll.scrollbar_state =
        list_scrollbar_state(content_len, view_height, scroll_offset);
    let title = issue_detail_title(issue);
    let keybinding_line = issue_detail_keybinding_line();
    Some(IssueDetailsView {
        layout,
        lines,
        title,
        keybinding_line,
        content_len,
        view_height,
        scroll_offset: scroll_offset.min(u16::MAX as usize) as u16,
        scrollbar_state: state.issue_details.scroll.scrollbar_state,
    })
}

fn issue_details_layout(area: Rect) -> IssueDetailsLayout {
    let width = (area.width.saturating_mul(80) / 100)
        .max(40)
        .min(area.width);
    let height = (area.height.saturating_mul(70) / 100)
        .max(10)
        .min(area.height);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    let popup = Rect::new(x, y, width, height);
    let inner = popup.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);
    IssueDetailsLayout {
        area: popup,
        content: sections[0],
        footer: sections[1],
    }
}

fn issue_detail_title(issue: &IssueRecord) -> Line<'static> {
    Line::from(Span::styled(
        format!("Issue {}", issue.id),
        Style::default().add_modifier(Modifier::BOLD),
    ))
}

fn issue_detail_keybinding_line() -> Line<'static> {
    let bindings = [
        ("j/k or Up/Down", "Scroll"),
        ("Alt+d", "Drop"),
        ("Esc/Enter", "Close"),
    ];
    keybinding_line_from_labels(&bindings, true)
}

fn issue_detail_lines(issue: &IssueRecord, current_username: &str) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut status_spans = vec![
        Span::styled("Status: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(
            issue_status_label(issue.status),
            issue_status_style(issue.status),
        ),
    ];
    if should_render_creator(current_username, &issue.creator) {
        status_spans.push(Span::raw(" "));
        status_spans.push(Span::styled(
            format!("@{}", issue.creator.as_str()),
            issue_creator_style(),
        ));
    }
    lines.push(Line::from(status_spans));
    if let Some(assignee) = issue
        .assignee
        .as_deref()
        .filter(|assignee| !assignee.is_empty())
    {
        lines.push(Line::from(vec![
            Span::styled("Assignee: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(format!("@{assignee}")),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Prompt",
        Style::default().add_modifier(Modifier::BOLD),
    )));
    append_issue_detail_text(
        &mut lines,
        issue.description.trim(),
        "No description provided.",
    );
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Issue log",
        Style::default().add_modifier(Modifier::BOLD),
    )));
    append_issue_detail_text(
        &mut lines,
        issue.progress.trim(),
        "No progress updates yet.",
    );
    lines
}

fn append_issue_detail_text(lines: &mut Vec<Line<'static>>, text: &str, empty_label: &str) {
    if text.is_empty() {
        lines.push(Line::from(Span::styled(
            empty_label.to_string(),
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for line in text.lines() {
            lines.push(Line::from(Span::raw(line.to_string())));
        }
    }
}

fn issue_prefix(depth: usize, has_children: bool, collapsed: bool) -> String {
    let base = if depth == 0 {
        "|".to_string()
    } else {
        format!("│{}", "  ".repeat(depth))
    };
    format!("{base}{}", issue_tree_indicator(has_children, collapsed))
}

fn issue_tree_indicator(has_children: bool, collapsed: bool) -> &'static str {
    if !has_children {
        " "
    } else if collapsed {
        "+"
    } else {
        "-"
    }
}

async fn refresh_jobs(
    client: &dyn MetisClientInterface,
    state: &mut DashboardState,
) -> Result<bool> {
    let response = client.list_jobs(&SearchJobsQuery::default()).await?;
    let now = Utc::now();

    let mut jobs = Vec::new();
    for summary in response.jobs {
        let issue_id = summary.task.spawned_from.clone();
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
    let (issues, repositories) =
        tokio::try_join!(fetch_issues(client), fetch_repositories(client))?;

    let issues_changed = issues != state.issues;
    if issues_changed {
        state.issues = issues;
    }
    let repositories_changed = repositories != state.repositories;
    if repositories_changed {
        state.repositories = repositories;
    }

    let derived_changed = update_views(state);
    Ok(issues_changed || repositories_changed || derived_changed)
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

async fn fetch_repositories(client: &dyn MetisClientInterface) -> Result<Vec<RepositoryRecord>> {
    let response = client
        .list_repositories(&SearchRepositoriesQuery::default())
        .await
        .context("failed to fetch repositories")?;
    Ok(response.repositories)
}

fn issue_to_record(record: ApiIssueRecord) -> Option<IssueRecord> {
    let issue = record.issue;
    Some(IssueRecord {
        id: record.id,
        issue_type: issue.issue_type,
        description: issue.description,
        creator: issue.creator,
        progress: issue.progress,
        status: issue.status,
        assignee: issue.assignee,
        dependencies: issue.dependencies,
        patches: issue.patches,
    })
}

fn update_views(state: &mut DashboardState) -> bool {
    let issue_details_open = state.issue_details.is_open;

    let known_issue_ids: HashSet<IssueId> =
        state.issues.iter().map(|issue| issue.id.clone()).collect();
    state
        .collapsed_issue_ids
        .retain(|issue_id| known_issue_ids.contains(issue_id));

    let filtered_issues = filter_issue_records(
        &state.issues,
        state.username.as_str(),
        state.issue_list_filter,
    );
    seed_completed_issue_collapses(state, &filtered_issues);

    let new_issue_lines = build_issue_lines_with_collapsed(
        &filtered_issues,
        &state.jobs,
        true,
        &state.collapsed_issue_ids,
    );
    let new_user_unowned_issue_lines =
        build_user_unowned_issue_lines(state.username.as_str(), &state.issues, &state.jobs);
    let new_completed_issue_lines = build_completed_issue_lines_with_collapsed(
        &filtered_issues,
        &state.jobs,
        &state.collapsed_issue_ids,
    );

    let mut changed = new_issue_lines != state.issue_lines
        || new_user_unowned_issue_lines != state.user_unowned_issue_lines
        || new_completed_issue_lines != state.completed_issue_lines;

    state.issue_lines = new_issue_lines;
    state.user_unowned_issue_lines = new_user_unowned_issue_lines;
    state.completed_issue_lines = new_completed_issue_lines;

    changed |= update_assignee_options(state);
    changed |= update_repo_options(state);

    clamp_issue_selections(state);
    update_panel_focus(state);
    clamp_issue_scrolls(state);
    reconcile_issue_details(state);

    changed || issue_details_open != state.issue_details.is_open
}

fn reconcile_issue_details(state: &mut DashboardState) {
    if !state.issue_details.is_open {
        return;
    }

    let Some(issue_id) = state.issue_details.issue_id.clone() else {
        close_issue_details(state);
        return;
    };

    if state.issues.iter().all(|issue| issue.id != issue_id) {
        close_issue_details(state);
    }
}

fn seed_completed_issue_collapses(state: &mut DashboardState, issues: &[IssueRecord]) {
    let nodes = build_issue_nodes(issues, &state.jobs);
    if nodes.is_empty() {
        state.known_completed_issue_ids.clear();
        return;
    }

    let completed_issue_ids = completed_issue_tree_ids(&nodes);
    for issue_id in &completed_issue_ids {
        if state.known_completed_issue_ids.contains(issue_id) {
            continue;
        }
        if nodes
            .get(issue_id)
            .is_some_and(|node| !node.children.is_empty())
        {
            state.collapsed_issue_ids.insert(issue_id.clone());
        }
    }
    state.known_completed_issue_ids = completed_issue_ids;
}

fn filter_issue_records(
    issues: &[IssueRecord],
    username: &str,
    issue_list_filter: IssueListFilter,
) -> Vec<IssueRecord> {
    match issue_list_filter {
        IssueListFilter::All => issues.to_vec(),
        IssueListFilter::CreatorOnly => {
            let trimmed_username = username.trim();
            if trimmed_username.is_empty() {
                return Vec::new();
            }
            issues
                .iter()
                .filter(|issue| issue.creator.as_str().trim() == trimmed_username)
                .cloned()
                .collect()
        }
    }
}

fn clamp_issue_selections(state: &mut DashboardState) {
    let running_len = state.issue_lines.rows.len();
    if running_len == 0 {
        state.running_issue_selection.index = 0;
    } else {
        state.running_issue_selection.index = state
            .running_issue_selection
            .index
            .min(running_len.saturating_sub(1));
    }

    let user_owned_len = state.user_unowned_issue_lines.rows.len();
    if user_owned_len == 0 {
        state.user_unowned_issue_selection.index = 0;
    } else {
        state.user_unowned_issue_selection.index = state
            .user_unowned_issue_selection
            .index
            .min(user_owned_len.saturating_sub(1));
    }

    let completed_len = completed_issue_count(&state.completed_issue_lines);
    if completed_len == 0 {
        state.completed_issue_selection.index = 0;
    } else {
        state.completed_issue_selection.index = state
            .completed_issue_selection
            .index
            .min(completed_len.saturating_sub(1));
    }
}

fn update_assignee_options(state: &mut DashboardState) -> bool {
    let fallback = "pm";
    let preferred = state
        .issue_draft
        .selected_assignee()
        .unwrap_or(fallback)
        .to_string();
    let new_options = build_assignee_options(&state.issues);
    let options_changed = new_options != state.issue_draft.assignees;
    if options_changed {
        state.issue_draft.assignees = new_options;
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
    let index_changed = next_index != state.issue_draft.assignee_index;
    state.issue_draft.assignee_index = next_index;
    options_changed || index_changed
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

fn update_repo_options(state: &mut DashboardState) -> bool {
    let preferred = state.issue_draft.selected_repo().cloned();
    let new_options = build_repo_options(&state.repositories);
    let options_changed = new_options != state.issue_draft.repos;
    if options_changed {
        state.issue_draft.repos = new_options;
    }

    let fallback_index = if state.issue_draft.repos.is_empty() {
        None
    } else {
        Some(0)
    };
    let next_index = preferred
        .and_then(|preferred| {
            state
                .issue_draft
                .repos
                .iter()
                .position(|repo| repo == &preferred)
        })
        .or(fallback_index)
        .unwrap_or(0);
    let index_changed = next_index != state.issue_draft.repo_index;
    state.issue_draft.repo_index = next_index;
    options_changed || index_changed
}

fn build_repo_options(repositories: &[RepositoryRecord]) -> Vec<RepoName> {
    let mut options = BTreeSet::new();
    for repository in repositories {
        options.insert(repository.name.clone());
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

    let collapsed_issue_ids = HashSet::new();
    let mut lines = build_issue_lines_with_collapsed(&assigned, jobs, false, &collapsed_issue_ids);
    for row in &mut lines.rows {
        row.depth = 0;
    }
    lines
}

#[cfg(test)]
fn build_issue_lines(
    issues: &[IssueRecord],
    jobs: &[JobDetails],
    exclude_inactive_roots: bool,
) -> IssueLines {
    let collapsed_issue_ids = HashSet::new();
    build_issue_lines_with_collapsed(issues, jobs, exclude_inactive_roots, &collapsed_issue_ids)
}

fn build_issue_lines_with_collapsed(
    issues: &[IssueRecord],
    jobs: &[JobDetails],
    exclude_inactive_roots: bool,
    collapsed_issue_ids: &HashSet<IssueId>,
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
        append_issue(
            &root,
            0,
            &mut rows,
            &mut visited,
            &nodes,
            collapsed_issue_ids,
        );
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

fn completed_issue_tree_ids(nodes: &HashMap<IssueId, IssueNode>) -> HashSet<IssueId> {
    let mut roots: Vec<IssueId> = nodes
        .iter()
        .filter(|(_, node)| node.parent.is_none())
        .map(|(id, _)| id.clone())
        .collect();
    roots.sort_by(|a, b| compare_issue_nodes(nodes, a, b));

    let mut completed_ids = HashSet::new();
    for root in roots {
        if !is_completed_tree(&root, nodes, &mut HashSet::new()) {
            continue;
        }
        collect_issue_ids(&root, nodes, &mut completed_ids);
    }

    completed_ids
}

fn collect_issue_ids(
    id: &IssueId,
    nodes: &HashMap<IssueId, IssueNode>,
    collected: &mut HashSet<IssueId>,
) {
    if !collected.insert(id.clone()) {
        return;
    }

    let Some(node) = nodes.get(id) else {
        return;
    };

    for child in &node.children {
        collect_issue_ids(child, nodes, collected);
    }
}

#[cfg(test)]
fn build_completed_issue_lines(issues: &[IssueRecord], jobs: &[JobDetails]) -> CompletedIssueLines {
    let collapsed_issue_ids = HashSet::new();
    build_completed_issue_lines_with_collapsed(issues, jobs, &collapsed_issue_ids)
}

fn build_completed_issue_lines_with_collapsed(
    issues: &[IssueRecord],
    jobs: &[JobDetails],
    collapsed_issue_ids: &HashSet<IssueId>,
) -> CompletedIssueLines {
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
        collect_issue_lines(
            &root,
            0,
            &mut lines,
            &mut visited,
            &nodes,
            collapsed_issue_ids,
        );
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
    collapsed_issue_ids: &HashSet<IssueId>,
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
        creator: node.record.creator.clone(),
        assignee: node.record.assignee.clone(),
        task: node.task.clone(),
        depth,
        has_children: !node.children.is_empty(),
        collapsed: collapsed_issue_ids.contains(id),
    });

    if collapsed_issue_ids.contains(id) {
        return;
    }

    let mut children = node.children.clone();
    children.sort_by(|a, b| compare_issue_nodes(nodes, a, b));
    for child in children {
        append_issue(&child, depth + 1, rows, visited, nodes, collapsed_issue_ids);
    }
}

fn collect_issue_lines(
    id: &IssueId,
    depth: usize,
    rows: &mut Vec<IssueLine>,
    visited: &mut HashSet<IssueId>,
    nodes: &HashMap<IssueId, IssueNode>,
    collapsed_issue_ids: &HashSet<IssueId>,
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
        creator: node.record.creator.clone(),
        assignee: node.record.assignee.clone(),
        task: node.task.clone(),
        depth,
        has_children: !node.children.is_empty(),
        collapsed: collapsed_issue_ids.contains(id),
    });

    if collapsed_issue_ids.contains(id) {
        return;
    }

    let mut children = node.children.clone();
    children.sort_by(|a, b| compare_issue_nodes(nodes, a, b));
    for child in children {
        collect_issue_lines(&child, depth + 1, rows, visited, nodes, collapsed_issue_ids);
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
                Status::Created
                | Status::Pending
                | Status::Running
                | Status::Complete
                | Status::Failed => job.runtime.clone(),
                _ => None,
            },
        })
}

fn task_status_order(status: Status) -> usize {
    match status {
        Status::Running => 0,
        Status::Pending => 1,
        Status::Created => 2,
        Status::Failed => 3,
        Status::Complete => 4,
        _ => 5,
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
        Status::Pending => Style::default().fg(Color::Cyan),
        Status::Created => Style::default().fg(Color::Blue),
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

fn issue_status_label(status: IssueStatus) -> &'static str {
    match status {
        IssueStatus::Open => "open",
        IssueStatus::InProgress => "in-progress",
        IssueStatus::Closed => "closed",
        IssueStatus::Dropped => "dropped",
        _ => "unknown",
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

fn open_issue_details(state: &mut DashboardState, issue_id: IssueId) {
    state.issue_details.is_open = true;
    state.issue_details.issue_id = Some(issue_id);
    state.issue_details.scroll = ListScrollState::default();
    state.issue_details.confirm_drop = false;
}

fn close_issue_details(state: &mut DashboardState) {
    state.issue_details.is_open = false;
    state.issue_details.issue_id = None;
    state.issue_details.scroll = ListScrollState::default();
    state.issue_details.confirm_drop = false;
}

fn scroll_issue_details(state: &mut DashboardState, delta: i32) -> bool {
    let Some(issue_id) = state.issue_details.issue_id.clone() else {
        return false;
    };
    let Some(issue) = state.issues.iter().find(|issue| issue.id == issue_id) else {
        return false;
    };
    let Some(size) = state.last_frame_size else {
        return false;
    };
    let layout = issue_details_layout(size);
    let lines = issue_detail_lines(issue, state.username.as_str());
    let view_height = layout.content.height as usize;
    let content_len = wrapped_content_len(&lines, layout.content.width);
    let max_offset = max_scroll_offset(content_len, view_height);
    let current = state.issue_details.scroll.offset;
    let next = if delta < 0 {
        current.saturating_sub(delta.unsigned_abs() as usize)
    } else {
        current.saturating_add(delta as usize)
    }
    .min(max_offset);
    if next == current {
        return false;
    }

    state.issue_details.scroll.offset = next;
    state.issue_details.scroll.scrollbar_state =
        list_scrollbar_state(content_len, view_height, next);
    true
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
        Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent,
        MouseEventKind,
    };
    use httpmock::prelude::*;
    use metis_common::issues::UpsertIssueResponse;
    use metis_common::jobs::{BundleSpec, Task};
    use metis_common::task_status::Event;
    use metis_common::{RepoName, Repository, RepositoryRecord};
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::prelude::StatefulWidget;
    use ratatui::Terminal;
    use serde_json::json;
    use std::collections::{HashMap, HashSet};
    use std::str::FromStr;
    use tui_textarea::CursorMove;

    const TEST_METIS_TOKEN: &str = "test-metis-token";

    fn job_with_status(id: &str, status: Status, offset_seconds: i64) -> JobRecord {
        let now = Utc::now() - ChronoDuration::seconds(offset_seconds);
        let mut log = TaskStatusLog::new(Status::Created, now);
        match status {
            Status::Pending => log.events.push(Event::Created {
                at: now,
                status: Status::Pending,
            }),
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
            Status::Created => {}
            other => unreachable!("unsupported task status variant: {other:?}"),
        }

        JobRecord::new(
            task_id(id),
            Task::new(
                "0".into(),
                BundleSpec::None,
                None,
                None,
                None,
                HashMap::new(),
                None,
                None,
                None,
                false,
            ),
            log,
        )
    }

    fn repo_record(name: &str) -> RepositoryRecord {
        RepositoryRecord::new(
            RepoName::from_str(name).expect("invalid repo name"),
            Repository::new("git@github.com:example/repo.git".to_string(), None, None),
        )
    }

    fn issue(id: &str, status: IssueStatus, dependencies: Vec<IssueDependency>) -> IssueRecord {
        IssueRecord {
            id: issue_id(id),
            issue_type: IssueType::Task,
            description: id.to_string(),
            creator: Username::from("alice"),
            progress: String::new(),
            status,
            assignee: None,
            dependencies,
            patches: Vec::new(),
        }
    }

    fn issue_with_assignee(id: &str, status: IssueStatus, assignee: Option<&str>) -> IssueRecord {
        IssueRecord {
            id: issue_id(id),
            issue_type: IssueType::Task,
            description: id.to_string(),
            creator: Username::from("alice"),
            progress: String::new(),
            status,
            assignee: assignee.map(str::to_string),
            dependencies: Vec::new(),
            patches: Vec::new(),
        }
    }

    fn issue_with_type(
        id: &str,
        issue_type: IssueType,
        status: IssueStatus,
        dependencies: Vec<IssueDependency>,
    ) -> IssueRecord {
        IssueRecord {
            id: issue_id(id),
            issue_type,
            description: id.to_string(),
            creator: Username::from("alice"),
            progress: String::new(),
            status,
            assignee: None,
            dependencies,
            patches: Vec::new(),
        }
    }

    fn child_of(issue_ref: &str) -> IssueDependency {
        IssueDependency::new(IssueDependencyType::ChildOf, issue_id(issue_ref))
    }

    fn blocked_on(issue_ref: &str) -> IssueDependency {
        IssueDependency::new(IssueDependencyType::BlockedOn, issue_id(issue_ref))
    }

    fn line_text(line: &Line<'_>) -> String {
        let mut text = String::new();
        for span in &line.spans {
            text.push_str(span.content.as_ref());
        }
        text
    }

    #[test]
    fn dashboard_title_includes_username_when_present() {
        assert_eq!(
            dashboard_title("cprussin", ""),
            "Metis Dashboard — cprussin"
        );
    }

    #[test]
    fn dashboard_title_skips_username_when_blank() {
        assert_eq!(
            dashboard_title(" ", "https://example.com"),
            "Metis Dashboard"
        );
    }

    #[test]
    fn dashboard_title_appends_server_url_when_present() {
        assert_eq!(
            dashboard_title("cprussin", "https://example.com/"),
            "Metis Dashboard — cprussin @ https://example.com"
        );
    }

    #[test]
    fn dashboard_header_hint_excludes_scroll() {
        let width = 120u16;
        let height = 1u16;
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal init failed");

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, width, height);
                render_dashboard_header(
                    frame,
                    area,
                    "metis-user",
                    "https://example.com",
                    IssueListFilter::All,
                );
            })
            .expect("draw failed");

        let buffer = terminal.backend().buffer();
        let header = row_text(buffer, 0, width);
        assert!(header.contains("Tab/Shift+Tab to change panels"));
        assert!(header.contains("Alt+F to filter issues"));
        assert!(header.contains("Ctrl+C to exit"));
        assert!(!header.contains("j/k or Up/Down"));
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
    fn note_or_error_shows_error_reason() {
        let job = job_with_status("t-job-failed", Status::Failed, 0);

        let message = note_or_error(&job);

        assert!(message.contains("boom"));
    }

    #[test]
    fn github_pr_url_prefers_configured_url() {
        let github = GithubPr::new(
            "octo".to_string(),
            "metis".to_string(),
            42,
            None,
            None,
            Some(" https://example.com/pr/42 ".to_string()),
            None,
        );

        assert_eq!(github_pr_url(&github), "https://example.com/pr/42");
    }

    #[test]
    fn github_pr_url_falls_back_to_default() {
        let github = GithubPr::new(
            "octo".to_string(),
            "metis".to_string(),
            42,
            None,
            None,
            None,
            None,
        );

        assert_eq!(
            github_pr_url(&github),
            "https://github.com/octo/metis/pull/42"
        );
    }

    #[test]
    fn parse_browser_command_handles_quotes() {
        let (command, args) = parse_browser_command("open -a \"Google Chrome\"").unwrap();

        assert_eq!(command, "open");
        assert_eq!(args, vec!["-a".to_string(), "Google Chrome".to_string()]);
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
    fn issue_lines_hide_descendants_when_collapsed() {
        let issues = vec![
            issue("i-root", IssueStatus::Open, vec![]),
            issue("i-child", IssueStatus::Open, vec![child_of("i-root")]),
        ];
        let mut collapsed_issue_ids = HashSet::new();
        collapsed_issue_ids.insert(issue_id("i-root"));

        let lines = build_issue_lines_with_collapsed(&issues, &[], false, &collapsed_issue_ids);

        assert_eq!(lines.rows.len(), 1);
        assert_eq!(lines.rows[0].id, issue_id("i-root").to_string());
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
            issue_type: IssueType::Task,
            description: "investigate logs".into(),
            creator: Username::from("alice"),
            progress: "drafting tests".into(),
            status: IssueStatus::Open,
            assignee: None,
            dependencies: Vec::new(),
            patches: Vec::new(),
        }];

        let lines = build_issue_lines(&issues, &[], false);

        let line = lines.rows.first().expect("issue line missing");
        assert_eq!(line.summary, "investigate logs");
        assert_eq!(line.progress.as_deref(), Some("drafting tests"));
    }

    #[test]
    fn issue_detail_lines_include_status_and_log() {
        let issue = IssueRecord {
            id: issue_id("i-detail"),
            issue_type: IssueType::Task,
            description: "Investigate memory spike".into(),
            creator: Username::from("alice"),
            progress: "Noted repro steps".into(),
            status: IssueStatus::InProgress,
            assignee: Some("alice".to_string()),
            dependencies: Vec::new(),
            patches: Vec::new(),
        };

        let lines = issue_detail_lines(&issue, "alice");
        let text: String = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");

        assert!(text.contains("Status:"));
        assert!(text.contains("in-progress"));
        assert!(text.contains("Assignee:"));
        assert!(text.contains("@alice"));
        assert!(text.contains("Prompt"));
        assert!(text.contains("Investigate memory spike"));
        assert!(text.contains("Issue log"));
        assert!(text.contains("Noted repro steps"));
    }

    #[test]
    fn issue_detail_lines_show_creator_for_non_self() {
        let issue = IssueRecord {
            id: issue_id("i-detail"),
            issue_type: IssueType::Task,
            description: "Investigate memory spike".into(),
            creator: Username::from("alice"),
            progress: String::new(),
            status: IssueStatus::Open,
            assignee: None,
            dependencies: Vec::new(),
            patches: Vec::new(),
        };

        let lines = issue_detail_lines(&issue, "bob");
        let status_line = lines.first().expect("missing status line");

        assert!(line_text(status_line).contains("@alice"));
    }

    #[test]
    fn issue_detail_lines_skip_creator_for_self() {
        let issue = IssueRecord {
            id: issue_id("i-detail"),
            issue_type: IssueType::Task,
            description: "Investigate memory spike".into(),
            creator: Username::from("alice"),
            progress: String::new(),
            status: IssueStatus::Open,
            assignee: None,
            dependencies: Vec::new(),
            patches: Vec::new(),
        };

        let lines = issue_detail_lines(&issue, "alice");
        let status_line = lines.first().expect("missing status line");

        assert!(!line_text(status_line).contains("@alice"));
    }

    fn dashboard_state_with_issues(issue_count: usize) -> DashboardState {
        let issues: Vec<IssueRecord> = (0..issue_count)
            .map(|index| issue(&format!("i-{index}"), IssueStatus::Open, Vec::new()))
            .collect();
        let issue_lines = build_issue_lines(&issues, &[], false);
        let user_unowned_issue_lines = build_issue_lines(&issues, &[], false);
        DashboardState {
            issues,
            issue_lines,
            user_unowned_issue_lines,
            ..DashboardState::default()
        }
    }

    #[test]
    fn default_issue_list_filter_is_all() {
        let state = DashboardState::default();
        assert_eq!(state.issue_list_filter, IssueListFilter::All);
    }

    #[test]
    fn filter_issue_records_respects_creator_only_and_all() {
        let issues = vec![
            IssueRecord {
                id: issue_id("i-user"),
                issue_type: IssueType::Task,
                description: "User issue".to_string(),
                creator: Username::from("alice"),
                progress: String::new(),
                status: IssueStatus::Open,
                assignee: None,
                dependencies: Vec::new(),
                patches: Vec::new(),
            },
            IssueRecord {
                id: issue_id("i-other"),
                issue_type: IssueType::Task,
                description: "Other issue".to_string(),
                creator: Username::from("bob"),
                progress: String::new(),
                status: IssueStatus::Open,
                assignee: None,
                dependencies: Vec::new(),
                patches: Vec::new(),
            },
        ];

        let all = filter_issue_records(&issues, "alice", IssueListFilter::All);
        assert_eq!(all.len(), 2);

        let creator_only = filter_issue_records(&issues, "  alice  ", IssueListFilter::CreatorOnly);
        assert_eq!(creator_only.len(), 1);
        assert_eq!(creator_only[0].creator, Username::from("alice"));
    }

    #[test]
    fn alt_f_toggles_issue_list_filter() {
        let issues = vec![
            IssueRecord {
                id: issue_id("i-user"),
                issue_type: IssueType::Task,
                description: "User issue".to_string(),
                creator: Username::from("alice"),
                progress: String::new(),
                status: IssueStatus::Open,
                assignee: None,
                dependencies: Vec::new(),
                patches: Vec::new(),
            },
            IssueRecord {
                id: issue_id("i-other"),
                issue_type: IssueType::Task,
                description: "Other issue".to_string(),
                creator: Username::from("bob"),
                progress: String::new(),
                status: IssueStatus::Open,
                assignee: None,
                dependencies: Vec::new(),
                patches: Vec::new(),
            },
        ];
        let mut state = DashboardState {
            issues,
            username: Username::from("alice"),
            ..DashboardState::default()
        };

        update_views(&mut state);
        assert_eq!(state.issue_list_filter, IssueListFilter::All);
        assert_eq!(state.issue_lines.rows.len(), 2);

        let outcome = handle_event(
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT)),
            &mut state,
        );

        assert!(!outcome.should_quit);
        assert_eq!(state.issue_list_filter, IssueListFilter::CreatorOnly);
        assert_eq!(state.issue_lines.rows.len(), 1);
        assert_eq!(state.issue_lines.rows[0].creator, Username::from("alice"));

        let outcome = handle_event(
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT)),
            &mut state,
        );

        assert!(!outcome.should_quit);
        assert_eq!(state.issue_list_filter, IssueListFilter::All);
        assert_eq!(state.issue_lines.rows.len(), 2);
    }

    #[test]
    fn status_panel_keys_move_issue_selection() {
        let mut state = dashboard_state_with_issues(3);
        state.selected_panel = PanelFocus::Running;

        let moved =
            handle_status_panel_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &mut state);
        assert!(moved);
        assert_eq!(state.running_issue_selection.index, 1);

        let moved = handle_status_panel_key(
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
            &mut state,
        );
        assert!(moved);
        assert_eq!(state.running_issue_selection.index, 2);

        let moved =
            handle_status_panel_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &mut state);
        assert!(!moved);
        assert_eq!(state.running_issue_selection.index, 2);
    }

    #[test]
    fn enter_on_user_owned_issue_requests_pr_open() {
        let mut state = dashboard_state_with_issues(1);
        state.selected_panel = PanelFocus::UserOwned;

        let outcome = handle_event(
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            &mut state,
        );

        assert_eq!(outcome.open_issue_pr, Some(issue_id("i-0")));
        assert!(outcome.submission.is_none());
    }

    #[test]
    fn enter_on_running_merge_request_requests_pr_open() {
        let issue = issue_with_type(
            "i-merge",
            IssueType::MergeRequest,
            IssueStatus::Open,
            Vec::new(),
        );
        let issue_lines = build_issue_lines(&[issue.clone()], &[], false);
        let mut state = DashboardState {
            issues: vec![issue],
            issue_lines,
            ..DashboardState::default()
        };
        state.selected_panel = PanelFocus::Running;

        let outcome = handle_event(
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            &mut state,
        );

        assert_eq!(outcome.open_issue_pr, Some(issue_id("i-merge")));
    }

    #[test]
    fn enter_on_completed_merge_request_requests_pr_open() {
        let issue = issue_with_type(
            "i-merge",
            IssueType::MergeRequest,
            IssueStatus::Closed,
            Vec::new(),
        );
        let completed_issue_lines = build_completed_issue_lines(&[issue.clone()], &[]);
        let mut state = DashboardState {
            issues: vec![issue],
            completed_issue_lines,
            ..DashboardState::default()
        };
        state.selected_panel = PanelFocus::Completed;

        let outcome = handle_event(
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            &mut state,
        );

        assert_eq!(outcome.open_issue_pr, Some(issue_id("i-merge")));
    }

    #[test]
    fn enter_on_running_non_merge_request_opens_details() {
        let issue = issue("i-0", IssueStatus::Open, Vec::new());
        let issue_lines = build_issue_lines(&[issue.clone()], &[], false);
        let mut state = DashboardState {
            issues: vec![issue],
            issue_lines,
            ..DashboardState::default()
        };
        state.selected_panel = PanelFocus::Running;

        let outcome = handle_event(
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            &mut state,
        );

        assert!(outcome.open_issue_pr.is_none());
        assert!(state.issue_details.is_open);
        assert_eq!(state.issue_details.issue_id, Some(issue_id("i-0")));
    }

    #[test]
    fn enter_on_completed_non_merge_request_opens_details() {
        let issue = issue("i-closed", IssueStatus::Closed, Vec::new());
        let completed_issue_lines = build_completed_issue_lines(&[issue.clone()], &[]);
        let mut state = DashboardState {
            issues: vec![issue],
            completed_issue_lines,
            ..DashboardState::default()
        };
        state.selected_panel = PanelFocus::Completed;

        let outcome = handle_event(
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            &mut state,
        );

        assert!(outcome.open_issue_pr.is_none());
        assert!(state.issue_details.is_open);
        assert_eq!(state.issue_details.issue_id, Some(issue_id("i-closed")));
    }

    #[test]
    fn escape_closes_issue_details() {
        let issue = issue("i-0", IssueStatus::Open, Vec::new());
        let issue_lines = build_issue_lines(&[issue.clone()], &[], false);
        let mut state = DashboardState {
            issues: vec![issue],
            issue_lines,
            ..DashboardState::default()
        };
        state.selected_panel = PanelFocus::Running;
        open_issue_details(&mut state, issue_id("i-0"));

        let outcome = handle_event(
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            &mut state,
        );

        assert!(!outcome.should_quit);
        assert!(state.issue_details.issue_id.is_none());
        assert!(!state.issue_details.is_open);
    }

    #[test]
    fn alt_d_opens_drop_confirmation() {
        let issue = issue("i-0", IssueStatus::Open, Vec::new());
        let issue_lines = build_issue_lines(&[issue.clone()], &[], false);
        let mut state = DashboardState {
            issues: vec![issue],
            issue_lines,
            ..DashboardState::default()
        };
        state.selected_panel = PanelFocus::Running;
        open_issue_details(&mut state, issue_id("i-0"));

        let outcome = handle_event(
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::ALT)),
            &mut state,
        );

        assert!(outcome.status_update.is_none());
        assert!(state.issue_details.confirm_drop);
        assert!(state.issue_details.is_open);
    }

    #[test]
    fn escape_cancels_drop_confirmation_without_closing_details() {
        let issue = issue("i-0", IssueStatus::Open, Vec::new());
        let issue_lines = build_issue_lines(&[issue.clone()], &[], false);
        let mut state = DashboardState {
            issues: vec![issue],
            issue_lines,
            ..DashboardState::default()
        };
        state.selected_panel = PanelFocus::Running;
        open_issue_details(&mut state, issue_id("i-0"));
        state.issue_details.confirm_drop = true;

        let outcome = handle_event(
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            &mut state,
        );

        assert!(outcome.status_update.is_none());
        assert!(!state.issue_details.confirm_drop);
        assert!(state.issue_details.is_open);
    }

    #[test]
    fn confirm_drop_emits_status_update() {
        let issue = issue("i-0", IssueStatus::Open, Vec::new());
        let issue_lines = build_issue_lines(&[issue.clone()], &[], false);
        let mut state = DashboardState {
            issues: vec![issue],
            issue_lines,
            ..DashboardState::default()
        };
        state.selected_panel = PanelFocus::Running;
        open_issue_details(&mut state, issue_id("i-0"));
        state.issue_details.confirm_drop = true;

        let outcome = handle_event(
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE)),
            &mut state,
        );

        let update = outcome.status_update.expect("missing status update");
        assert_eq!(update.issue_id, issue_id("i-0"));
        assert_eq!(update.status, IssueStatus::Dropped);
        assert!(!state.issue_details.confirm_drop);
        assert!(state.issue_details.is_open);
    }

    #[test]
    fn toggle_selected_issue_children_keeps_selection() {
        let issues = vec![
            issue("i-root", IssueStatus::Open, vec![]),
            issue("i-child", IssueStatus::Open, vec![child_of("i-root")]),
        ];
        let mut state = DashboardState {
            issues,
            ..DashboardState::default()
        };
        update_views(&mut state);
        state.selected_panel = PanelFocus::Running;
        update_panel_focus(&mut state);

        let toggled = toggle_selected_issue_children(&mut state);

        assert!(toggled);
        assert_eq!(state.issue_lines.rows.len(), 1);
        assert_eq!(state.running_issue_selection.index, 0);
        assert_eq!(state.issue_lines.rows[0].id, issue_id("i-root").to_string());
    }

    #[test]
    fn space_toggles_issue_children_expansion() {
        let issues = vec![
            issue("i-root", IssueStatus::Open, vec![]),
            issue("i-child", IssueStatus::Open, vec![child_of("i-root")]),
        ];
        let mut state = DashboardState {
            issues,
            ..DashboardState::default()
        };
        state.selected_panel = PanelFocus::Running;
        update_panel_focus(&mut state);
        update_views(&mut state);

        assert_eq!(state.issue_lines.rows.len(), 2);

        let outcome = handle_event(
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)),
            &mut state,
        );

        assert!(!outcome.should_quit);
        assert!(outcome.submission.is_none());
        assert_eq!(state.issue_lines.rows.len(), 1);
        assert!(state.collapsed_issue_ids.contains(&issue_id("i-root")));

        let outcome = handle_event(
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)),
            &mut state,
        );

        assert!(!outcome.should_quit);
        assert!(outcome.submission.is_none());
        assert_eq!(state.issue_lines.rows.len(), 2);
        assert!(!state.collapsed_issue_ids.contains(&issue_id("i-root")));
    }

    #[test]
    fn space_does_not_toggle_when_issue_creator_focused() {
        let issues = vec![
            issue("i-root", IssueStatus::Open, vec![]),
            issue("i-child", IssueStatus::Open, vec![child_of("i-root")]),
        ];
        let mut state = DashboardState {
            issues,
            ..DashboardState::default()
        };
        update_views(&mut state);
        update_panel_focus(&mut state);

        let outcome = handle_event(
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)),
            &mut state,
        );

        assert!(!outcome.should_quit);
        assert!(outcome.submission.is_none());
        assert_eq!(state.issue_lines.rows.len(), 2);
        assert!(state.collapsed_issue_ids.is_empty());
    }

    #[test]
    fn space_does_not_toggle_without_issue_selection() {
        let mut state = DashboardState {
            selected_panel: PanelFocus::Running,
            ..DashboardState::default()
        };
        update_panel_focus(&mut state);
        update_views(&mut state);

        let outcome = handle_event(
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)),
            &mut state,
        );

        assert!(!outcome.should_quit);
        assert!(outcome.submission.is_none());
        assert!(state.issue_lines.rows.is_empty());
        assert!(state.collapsed_issue_ids.is_empty());
    }

    #[test]
    fn status_panel_selection_scrolls_into_view() {
        let mut state = dashboard_state_with_issues(12);
        let size = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 30,
        };
        state.last_frame_size = Some(size);
        state.selected_panel = PanelFocus::Running;
        update_panel_focus(&mut state);
        clamp_issue_scrolls(&mut state);

        let layout = dashboard_layout(size);
        let panels = issue_panel_layout(layout.issue_sections);
        let running_lines = issue_line_lines(
            &state.issue_lines.rows,
            "No issues found",
            true,
            state.username.as_str(),
            None,
            false,
        );
        let (_, view_height) = panel_scroll_metrics(panels.running, &running_lines);
        assert!(view_height > 0);

        for _ in 0..view_height {
            handle_status_panel_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &mut state);
        }

        assert_eq!(state.running_issue_selection.index, view_height);
        assert_eq!(state.running_issue_panel.scroll_offset(), 1);

        for _ in 0..view_height {
            handle_status_panel_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), &mut state);
        }

        assert_eq!(state.running_issue_selection.index, 0);
        assert_eq!(state.running_issue_panel.scroll_offset(), 0);
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
    fn completed_issue_trees_default_to_collapsed() {
        let issues = vec![
            issue("i-root", IssueStatus::Closed, vec![]),
            issue("i-child", IssueStatus::Closed, vec![child_of("i-root")]),
        ];
        let mut state = DashboardState {
            issues,
            ..DashboardState::default()
        };

        update_views(&mut state);

        assert!(state.collapsed_issue_ids.contains(&issue_id("i-root")));
        let descendants = state
            .completed_issue_lines
            .descendants
            .get(&issue_id("i-root"))
            .expect("missing descendants");
        assert!(descendants.is_empty());
    }

    #[test]
    fn completed_issue_expansion_persists_across_refresh() {
        let issues = vec![
            issue("i-root", IssueStatus::Closed, vec![]),
            issue("i-child", IssueStatus::Closed, vec![child_of("i-root")]),
        ];
        let mut state = DashboardState {
            issues,
            ..DashboardState::default()
        };

        update_views(&mut state);
        state.selected_panel = PanelFocus::Completed;
        update_panel_focus(&mut state);

        let toggled = toggle_selected_issue_children(&mut state);
        assert!(toggled);
        assert!(!state.collapsed_issue_ids.contains(&issue_id("i-root")));
        assert_eq!(completed_issue_rows(&state.completed_issue_lines).len(), 2);

        update_views(&mut state);
        assert!(!state.collapsed_issue_ids.contains(&issue_id("i-root")));
        assert_eq!(completed_issue_rows(&state.completed_issue_lines).len(), 2);
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
                issue_type: IssueType::Task,
                description: "i-root".to_string(),
                creator: Username::from("alice"),
                progress: String::new(),
                status: IssueStatus::Open,
                assignee: Some("alice".to_string()),
                dependencies: Vec::new(),
                patches: Vec::new(),
            },
            IssueRecord {
                id: issue_id("i-child"),
                issue_type: IssueType::Task,
                description: "i-child".to_string(),
                creator: Username::from("alice"),
                progress: String::new(),
                status: IssueStatus::Open,
                assignee: Some("alice".to_string()),
                dependencies: vec![child_of("i-root")],
                patches: Vec::new(),
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
            username: Username::from("alice"),
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
    fn issue_line_prefix_shows_collapsed_indicator() {
        let issues = vec![
            issue("i-root", IssueStatus::Open, vec![]),
            issue("i-child", IssueStatus::Open, vec![child_of("i-root")]),
        ];
        let mut collapsed = HashSet::new();
        collapsed.insert(issue_id("i-root"));

        let lines = build_issue_lines_with_collapsed(&issues, &[], false, &collapsed);
        let rendered = issue_line_lines(&lines.rows, "No issues found", true, "alice", None, false);

        assert!(line_text(&rendered[0]).starts_with("|+ "));
    }

    #[test]
    fn issue_line_prefix_shows_expanded_indicator() {
        let issues = vec![
            issue("i-root", IssueStatus::Open, vec![]),
            issue("i-child", IssueStatus::Open, vec![child_of("i-root")]),
        ];

        let lines = build_issue_lines(&issues, &[], false);
        let rendered = issue_line_lines(&lines.rows, "No issues found", true, "alice", None, false);

        assert!(line_text(&rendered[0]).starts_with("|- "));
    }

    #[test]
    fn issue_line_shows_creator_for_non_self() {
        let issues = vec![issue("i-root", IssueStatus::Open, vec![])];
        let lines = build_issue_lines(&issues, &[], false);
        let rendered = issue_line_lines(&lines.rows, "No issues found", true, "bob", None, false);

        assert!(line_text(&rendered[0]).contains("@alice"));
    }

    #[test]
    fn issue_line_hides_creator_for_self() {
        let issues = vec![issue("i-root", IssueStatus::Open, vec![])];
        let lines = build_issue_lines(&issues, &[], false);
        let rendered = issue_line_lines(&lines.rows, "No issues found", true, "alice", None, false);

        assert!(!line_text(&rendered[0]).contains("@alice"));
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
    fn build_repo_options_includes_unique_sorted() {
        let repositories = vec![
            repo_record("dourolabs/metis"),
            repo_record("dourolabs/api"),
            repo_record("dourolabs/metis"),
        ];

        let options = build_repo_options(&repositories);

        assert_eq!(
            options,
            vec![
                RepoName::from_str("dourolabs/api").unwrap(),
                RepoName::from_str("dourolabs/metis").unwrap(),
            ]
        );
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
    fn update_repo_options_keeps_existing_selection() {
        let mut state = DashboardState::default();
        state.issue_draft.repos = vec![
            RepoName::from_str("dourolabs/metis").unwrap(),
            RepoName::from_str("dourolabs/api").unwrap(),
        ];
        state.issue_draft.repo_index = 1;
        state.repositories = vec![repo_record("dourolabs/metis"), repo_record("dourolabs/api")];

        update_repo_options(&mut state);

        assert_eq!(
            state.issue_draft.selected_repo(),
            Some(&RepoName::from_str("dourolabs/api").unwrap())
        );
    }

    #[test]
    fn update_repo_options_defaults_to_first_option() {
        let mut state = DashboardState {
            repositories: vec![repo_record("dourolabs/metis")],
            ..DashboardState::default()
        };

        update_repo_options(&mut state);

        assert_eq!(
            state.issue_draft.selected_repo(),
            Some(&RepoName::from_str("dourolabs/metis").unwrap())
        );
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
        state.issue_draft.repos = vec![RepoName::from_str("dourolabs/metis").unwrap()];

        let submission = attempt_issue_submit(&mut state).expect("submission missing");

        assert_eq!(submission.prompt, "Ship dashboard");
        assert_eq!(submission.assignee, "pm");
        assert_eq!(
            submission.repo_name,
            Some(RepoName::from_str("dourolabs/metis").unwrap())
        );
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
    fn alt_r_cycles_repo_in_new_issue_panel() {
        let mut state = DashboardState::default();
        state.issue_draft.repos = vec![
            RepoName::from_str("dourolabs/metis").unwrap(),
            RepoName::from_str("dourolabs/api").unwrap(),
        ];

        let outcome = handle_event(
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::ALT)),
            &mut state,
        );

        assert!(!outcome.should_quit);
        assert!(outcome.submission.is_none());
        assert_eq!(
            state.issue_draft.selected_repo(),
            Some(&RepoName::from_str("dourolabs/api").unwrap())
        );
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
    fn alt_r_does_not_cycle_repo_when_not_focused() {
        let mut state = DashboardState {
            selected_panel: PanelFocus::Running,
            ..DashboardState::default()
        };
        state.issue_draft.repos = vec![
            RepoName::from_str("dourolabs/metis").unwrap(),
            RepoName::from_str("dourolabs/api").unwrap(),
        ];

        let outcome = handle_event(
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::ALT)),
            &mut state,
        );

        assert!(!outcome.should_quit);
        assert!(outcome.submission.is_none());
        assert_eq!(
            state.issue_draft.selected_repo(),
            Some(&RepoName::from_str("dourolabs/metis").unwrap())
        );
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
    fn wrapped_issue_lines_select_with_keyboard_input() {
        let long_description = "x".repeat(200);
        let long_issue = IssueRecord {
            description: long_description,
            ..issue("i-long", IssueStatus::Open, vec![])
        };
        let second = issue("i-short", IssueStatus::Open, vec![]);
        let mut state = DashboardState {
            issues: vec![long_issue, second],
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
        assert!(state.running_issue_panel.scroll_offset() > 0);
        assert_eq!(state.running_issue_selection.index, 1);
    }

    #[test]
    fn issue_draft_prompt_metrics_wraps_cursor_row() {
        let mut textarea = TextArea::from(["012345"]);
        textarea.move_cursor(CursorMove::End);

        let (total, cursor_row) = issue_draft_prompt_metrics(&textarea, 4);

        assert_eq!(total, 2);
        assert_eq!(cursor_row, 1);
    }

    #[test]
    fn issue_draft_prompt_lines_wraps_long_line() {
        let textarea = TextArea::from(["012345"]);

        let lines = issue_draft_prompt_lines(&textarea, 4);

        assert_eq!(lines.len(), 2);
        assert_eq!(line_text(&lines[0]), "0123");
        assert_eq!(line_text(&lines[1]), "45");
    }

    #[test]
    fn issue_draft_prompt_lines_show_cursor_at_line_end() {
        let mut textarea = TextArea::from(["abcd"]);
        textarea.move_cursor(CursorMove::End);

        let lines = issue_draft_prompt_lines(&textarea, 10);

        assert_eq!(lines.len(), 1);
        assert_eq!(line_text(&lines[0]), "abcd ");
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
            username: Username::from("alice"),
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
    #[allow(clippy::await_holding_lock)]
    async fn submit_issue_sends_task_request() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST).path("/v1/issues").json_body(json!({
                "issue": {
                    "type": "task",
                    "description": "Draft release notes",
                    "creator": " metis-user ",
                    "progress": "",
                    "status": "open",
                    "assignee": "alice",
                    "job_settings": {
                        "repo_name": "dourolabs/metis"
                    },
                    "dependencies": [],
                    "patches": []
                }
            }));
            then.status(200)
                .json_body_obj(&UpsertIssueResponse::new(issue_id("i-new")));
        });

        let client =
            MetisClient::new(server.base_url(), TEST_METIS_TOKEN).expect("failed to create client");

        let submission = IssueSubmission {
            prompt: "Draft release notes".to_string(),
            assignee: "alice".to_string(),
            repo_name: Some(RepoName::from_str("dourolabs/metis").unwrap()),
        };

        let created = submit_issue(&client, &submission, &Username::from(" metis-user "))
            .await
            .expect("submission failed");

        assert_eq!(created, issue_id("i-new"));
        mock.assert();
    }

    fn row_text(buffer: &Buffer, y: u16, width: u16) -> String {
        let mut row = String::new();
        for x in 0..width {
            row.push_str(buffer[(x, y)].symbol());
        }
        row.trim_end().to_string()
    }
}
