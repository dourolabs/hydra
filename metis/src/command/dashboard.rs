use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
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
    issues::{
        IssueDependency, IssueDependencyType, IssueRecord as ApiIssueRecord, IssueStatus,
        SearchIssuesQuery,
    },
    jobs::JobSummary,
    patches::{PatchRecord as ApiPatchRecord, SearchPatchesQuery},
    task_status::{Status, TaskError, TaskStatusLog},
    IssueId, MetisId, PatchId, TaskId,
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
const RECORD_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const MAX_RUNNING_JOBS: usize = 10;
const MAX_FINISHED_JOBS: usize = 12;
const MAX_PATCH_ROWS: usize = 12;
const MAX_MESSAGE_WIDTH: usize = 90;
const ISSUE_ID_VAR: &str = "METIS_ISSUE_ID";

#[derive(Default, Clone, PartialEq)]
struct JobSections {
    running: Vec<JobDisplay>,
    finished: Vec<JobDisplay>,
}

#[derive(Clone, PartialEq)]
struct JobDetails {
    display: JobDisplay,
    issue_id: Option<IssueId>,
    emitted_artifacts: Vec<MetisId>,
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
struct PatchDisplay {
    id: String,
    summary: String,
}

#[derive(Clone, PartialEq)]
struct IssueRecord {
    id: IssueId,
    description: String,
    status: IssueStatus,
    assignee: Option<String>,
    dependencies: Vec<IssueDependency>,
}

#[derive(Clone, PartialEq)]
struct PatchRecord {
    id: PatchId,
    summary: String,
}

#[derive(Default, Clone, PartialEq)]
struct IssueLines {
    rows: Vec<IssueLine>,
}

#[derive(Clone, PartialEq)]
struct IssueLine {
    id: String,
    summary: String,
    status: IssueStatus,
    readiness: IssueReadiness,
    assignee: Option<String>,
    patch_count: usize,
    task: Option<TaskIndicator>,
    depth: usize,
}

#[derive(Clone, PartialEq, Debug)]
enum IssueReadiness {
    Ready,
    Blocked(Vec<String>),
    Waiting,
}

#[derive(Clone)]
struct IssueNode {
    record: IssueRecord,
    parent: Option<IssueId>,
    children: Vec<IssueId>,
    patch_count: usize,
    task: Option<TaskIndicator>,
}

#[derive(Clone, PartialEq)]
struct TaskIndicator {
    status: Status,
    runtime: Option<String>,
}

#[derive(Default, Clone, PartialEq)]
struct DashboardState {
    jobs: Vec<JobDetails>,
    issues: Vec<IssueRecord>,
    patches: Vec<PatchRecord>,
    issue_lines: IssueLines,
    unassociated_jobs: JobSections,
    unassociated_patches: Vec<PatchDisplay>,
    jobs_error: Option<String>,
    records_error: Option<String>,
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

    match refresh_records(client, &mut state).await {
        Ok(changed) => needs_draw |= changed,
        Err(err) => {
            state.records_error = Some(format!("Failed to load issues and patches: {err}"));
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
                        state.records_error = Some(format!("Failed to refresh issues and patches: {err}"));
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
            Constraint::Length(3),
            Constraint::Min(12),
            Constraint::Length(10),
        ])
        .split(frame.size());

    render_header(frame, chunks[0], state);
    render_issues(frame, chunks[1], state);
    render_other_panels(frame, chunks[2], state);
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
            format!("Issues/Patches: {error}"),
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

fn render_issues(frame: &mut Frame, area: ratatui::layout::Rect, state: &DashboardState) {
    let items: Vec<ListItem> = if state.issue_lines.rows.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "No issues found",
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        state
            .issue_lines
            .rows
            .iter()
            .map(|line| {
                let mut spans = Vec::new();
                spans.push(Span::raw("  ".repeat(line.depth)));
                let (issue_status_label, issue_status_style) =
                    issue_status_display(line.status, &line.readiness);
                spans.push(Span::styled(
                    format!("[{issue_status_label}]"),
                    issue_status_style,
                ));
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    line.id.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ));
                if let Some(task) = &line.task {
                    spans.push(Span::raw(" "));
                    let mut task_label = format!("[task:{}", status_label(task.status));
                    if let Some(runtime) = &task.runtime {
                        task_label.push(' ');
                        task_label.push_str(runtime);
                    }
                    task_label.push(']');
                    spans.push(Span::styled(task_label, status_style(task.status)));
                }
                if line.patch_count > 0 {
                    spans.push(Span::raw(" "));
                    spans.push(Span::styled(
                        format!("[patch:{}]", line.patch_count),
                        Style::default().fg(Color::Cyan),
                    ));
                }
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

                ListItem::new(Line::from(spans))
            })
            .collect()
    };

    let block = Block::default()
        .title("Issues")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::White));
    frame.render_widget(List::new(items).block(block), area);
}

fn render_other_panels(frame: &mut Frame, area: ratatui::layout::Rect, state: &DashboardState) {
    let rows = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    render_unassociated_jobs(frame, rows[0], state);
    render_unassociated_patches(frame, rows[1], state);
}

fn render_unassociated_jobs(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    state: &DashboardState,
) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let running_table = job_table(
        "Other running tasks",
        &state.unassociated_jobs.running,
        &[
            Constraint::Percentage(28),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Min(10),
        ],
    );
    let finished_table = job_table(
        "Other recent results",
        &state.unassociated_jobs.finished,
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
                    Cell::from(job.id.to_string()),
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

fn render_unassociated_patches(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    state: &DashboardState,
) {
    let items: Vec<ListItem> = if state.unassociated_patches.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "No patches",
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        state
            .unassociated_patches
            .iter()
            .map(|patch| {
                ListItem::new(Line::from(vec![
                    Span::styled(&patch.id, Style::default().fg(Color::Cyan)),
                    Span::raw(" — "),
                    Span::raw(truncate_message(&patch.summary, MAX_MESSAGE_WIDTH)),
                ]))
            })
            .collect()
    };

    let block = Block::default()
        .title("Patches without issues")
        .borders(Borders::ALL);
    frame.render_widget(List::new(items).block(block), area);
}

async fn refresh_jobs(
    client: &dyn MetisClientInterface,
    state: &mut DashboardState,
) -> Result<bool> {
    let response = client.list_jobs().await?;
    let now = Utc::now();

    let previous_jobs = state.jobs.clone();
    let mut jobs = Vec::new();
    for summary in response.jobs {
        let issue_id = match cached_issue_id(&previous_jobs, &summary.id) {
            Some(id) => id,
            None => fetch_issue_id(client, &summary.id).await?,
        };
        let emitted_artifacts = summary.status_log.emitted_artifacts().unwrap_or_default();
        let display = summarize_job(summary, now);
        jobs.push(JobDetails {
            display,
            issue_id,
            emitted_artifacts,
        });
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
    let patches = fetch_patches(client).await?;

    let changed = issues != state.issues || patches != state.patches;
    if changed {
        state.issues = issues;
        state.patches = patches;
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

async fn fetch_patches(client: &dyn MetisClientInterface) -> Result<Vec<PatchRecord>> {
    let response = client
        .list_patches(&SearchPatchesQuery::default())
        .await
        .context("failed to fetch patches")?;

    let patches = response
        .patches
        .into_iter()
        .filter_map(patch_to_record)
        .collect();

    Ok(patches)
}

fn issue_to_record(record: ApiIssueRecord) -> Option<IssueRecord> {
    let issue = record.issue;
    Some(IssueRecord {
        id: record.id,
        description: issue.description,
        status: issue.status,
        assignee: issue.assignee,
        dependencies: issue.dependencies,
    })
}

fn patch_to_record(record: ApiPatchRecord) -> Option<PatchRecord> {
    let patch = record.patch;
    let summary = if patch.title.trim().is_empty() {
        patch.description
    } else {
        patch.title
    };
    Some(PatchRecord {
        id: record.id,
        summary,
    })
}

fn update_views(state: &mut DashboardState) -> bool {
    let previous_issue_lines = state.issue_lines.clone();
    let previous_unassociated_jobs = state.unassociated_jobs.clone();
    let previous_unassociated_patches = state.unassociated_patches.clone();

    let (issue_lines, associated_patch_ids) =
        build_issue_lines(&state.issues, &state.jobs, &state.patches);

    let unassociated_jobs = categorize_jobs(
        state
            .jobs
            .iter()
            .filter(|job| job.issue_id.is_none())
            .map(|job| job.display.clone())
            .collect(),
    );

    let mut unassociated_patches: Vec<PatchDisplay> = state
        .patches
        .iter()
        .filter(|patch| !associated_patch_ids.contains(&patch.id))
        .take(MAX_PATCH_ROWS)
        .map(|patch| PatchDisplay {
            id: patch.id.to_string(),
            summary: patch.summary.clone(),
        })
        .collect();
    unassociated_patches.truncate(MAX_PATCH_ROWS);

    state.issue_lines = issue_lines;
    state.unassociated_jobs = unassociated_jobs;
    state.unassociated_patches = unassociated_patches;

    previous_issue_lines != state.issue_lines
        || previous_unassociated_jobs != state.unassociated_jobs
        || previous_unassociated_patches != state.unassociated_patches
}

fn build_issue_lines(
    issues: &[IssueRecord],
    jobs: &[JobDetails],
    patches: &[PatchRecord],
) -> (IssueLines, HashSet<PatchId>) {
    let issue_ids: HashSet<IssueId> = issues.iter().map(|issue| issue.id.clone()).collect();
    let mut tasks_by_issue: HashMap<IssueId, Vec<JobDisplay>> = HashMap::new();
    let mut emitted_artifacts_by_issue: HashMap<IssueId, HashSet<MetisId>> = HashMap::new();

    for job in jobs {
        if let Some(issue_id) = &job.issue_id {
            tasks_by_issue
                .entry(issue_id.clone())
                .or_default()
                .push(job.display.clone());

            let emitted = emitted_artifacts_by_issue
                .entry(issue_id.clone())
                .or_default();
            for artifact_id in &job.emitted_artifacts {
                emitted.insert(artifact_id.clone());
            }
        }
    }

    let patch_lookup: HashSet<PatchId> = patches.iter().map(|patch| patch.id.clone()).collect();
    let mut patch_ids_by_issue: HashMap<IssueId, HashSet<PatchId>> = HashMap::new();
    let mut associated_patch_ids = HashSet::new();
    for (issue_id, artifact_ids) in &emitted_artifacts_by_issue {
        if !issue_ids.contains(issue_id) {
            continue;
        }
        for artifact_id in artifact_ids {
            if let Some(patch_id) = artifact_id.as_patch_id() {
                if patch_lookup.contains(&patch_id) {
                    patch_ids_by_issue
                        .entry(issue_id.clone())
                        .or_default()
                        .insert(patch_id.clone());
                    associated_patch_ids.insert(patch_id);
                }
            }
        }
    }

    let mut nodes: HashMap<IssueId, IssueNode> = issues
        .iter()
        .map(|issue| {
            let task = tasks_by_issue
                .get(&issue.id)
                .and_then(|tasks| best_task_indicator(tasks));
            let patch_count = patch_ids_by_issue
                .get(&issue.id)
                .map(|set| set.len())
                .unwrap_or(0);
            (
                issue.id.clone(),
                IssueNode {
                    record: issue.clone(),
                    parent: None,
                    children: Vec::new(),
                    patch_count,
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

    (IssueLines { rows }, associated_patch_ids)
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

    rows.push(IssueLine {
        id: node.record.id.to_string(),
        summary: node.record.description.clone(),
        status: node.record.status,
        readiness,
        assignee: node.record.assignee.clone(),
        patch_count: node.patch_count,
        task: node.task.clone(),
        depth,
    });

    let mut children = node.children.clone();
    children.sort_by(|a, b| compare_issue_nodes(nodes, a, b));
    for child in children {
        append_issue(&child, depth + 1, rows, visited, nodes);
    }
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
                Status::Running | Status::Complete => job.runtime.clone(),
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
        IssueStatus::Closed => 2,
    }
}

fn categorize_jobs(jobs: Vec<JobDisplay>) -> JobSections {
    let mut running = Vec::new();
    let mut finished = Vec::new();

    for job in jobs {
        match job.status {
            Status::Complete | Status::Failed => finished.push(job),
            _ => running.push(job),
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
        Status::Pending => Style::default().fg(Color::Blue),
    }
}

fn issue_status_style(status: IssueStatus) -> Style {
    match status {
        IssueStatus::Open => Style::default().fg(Color::Blue),
        IssueStatus::InProgress => Style::default().fg(Color::Yellow),
        IssueStatus::Closed => Style::default().fg(Color::Green),
    }
}

fn issue_status_display(status: IssueStatus, readiness: &IssueReadiness) -> (String, Style) {
    match (status, readiness) {
        (IssueStatus::Open, IssueReadiness::Blocked(blockers)) => (
            format!("blocked: {}", blockers.join(", ")),
            Style::default().fg(Color::Magenta),
        ),
        (IssueStatus::InProgress, IssueReadiness::Waiting) => {
            ("waiting".to_string(), Style::default().fg(Color::Magenta))
        }
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
    use crate::test_utils::ids::{issue_id, patch_id, task_id};
    use chrono::Duration as ChronoDuration;
    use metis_common::task_status::Event;

    fn job_with_status(id: &str, status: Status, offset_seconds: i64) -> JobSummary {
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

        JobSummary {
            id: task_id(id),
            notes: None,
            program: "0".into(),
            params: vec![],
            status_log: log,
        }
    }

    fn issue(id: &str, status: IssueStatus, dependencies: Vec<IssueDependency>) -> IssueRecord {
        IssueRecord {
            id: issue_id(id),
            description: id.to_string(),
            status,
            assignee: None,
            dependencies,
        }
    }

    fn job_details_with_issue(
        id: &str,
        status: Status,
        linked_issue: Option<&str>,
        emitted_artifacts: Vec<&str>,
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
            emitted_artifacts: emitted_artifacts
                .into_iter()
                .map(|value| MetisId::from(patch_id(value)))
                .collect(),
        }
    }

    #[test]
    fn categorize_jobs_splits_running_and_finished() {
        let now = Utc::now();
        let jobs = vec![
            summarize_job(job_with_status("t-job-running", Status::Running, 0), now),
            summarize_job(job_with_status("t-job-complete", Status::Complete, 1), now),
            summarize_job(job_with_status("t-job-failed", Status::Failed, 2), now),
        ];

        let categorized = categorize_jobs(jobs);

        assert_eq!(categorized.running.len(), 1);
        assert_eq!(categorized.finished.len(), 2);
        assert_eq!(categorized.running[0].id, task_id("t-job-running"));
        assert!(categorized
            .finished
            .iter()
            .any(|job| job.id == task_id("t-job-complete")));
        assert!(categorized
            .finished
            .iter()
            .any(|job| job.id == task_id("t-job-failed")));
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

        let (lines, _) = build_issue_lines(&issues, &[], &[]);

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

        let (lines, _) = build_issue_lines(&issues, &[], &[]);

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
    fn issue_lines_include_task_and_patch_indicators() {
        let issues = vec![issue("i-1", IssueStatus::Open, vec![])];
        let patches = vec![PatchRecord {
            id: patch_id("p-1"),
            summary: "fix".to_string(),
        }];
        let jobs = vec![job_details_with_issue(
            "t-job-1",
            Status::Running,
            Some("i-1"),
            vec!["p-1"],
            Some("3s"),
        )];

        let (lines, associated_patches) = build_issue_lines(&issues, &jobs, &patches);

        assert!(associated_patches.contains(&patch_id("p-1")));
        let line = lines.rows.first().expect("issue line missing");
        assert_eq!(line.patch_count, 1);
        let task = line.task.as_ref().expect("task indicator missing");
        assert_eq!(task.status, Status::Running);
        assert_eq!(task.runtime.as_deref(), Some("3s"));
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

        let (lines, _) = build_issue_lines(&issues, &[], &[]);

        let line = lines
            .rows
            .iter()
            .find(|line| line.id == issue_id("i-root").to_string())
            .expect("issue line missing");
        assert_eq!(line.status, IssueStatus::InProgress);
        assert!(matches!(line.readiness, IssueReadiness::Waiting));
    }
}
