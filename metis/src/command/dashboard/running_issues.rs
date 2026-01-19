use std::collections::{HashMap, HashSet};

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    Frame,
};

use super::panel::{Panel, PanelState};
use super::{
    build_issue_nodes, compare_issue_nodes, issue_prefix, issue_readiness, issue_status_display,
    issue_summary, status_style, truncate_message, IssueId, IssueLine, IssueLines, IssueNode,
    IssueRecord, IssueStatus, JobDetails, MAX_MESSAGE_WIDTH,
};

pub(super) fn render_running_issues_panel(
    frame: &mut Frame,
    area: Rect,
    issue_lines: &IssueLines,
    panel_state: &mut PanelState,
) {
    let running_title = issue_list_title("Running issues", issue_lines);
    let running_lines = issue_line_lines(&issue_lines.rows, "No issues found");
    let running_panel = Panel::new(Line::from(running_title), running_lines);
    frame.render_stateful_widget(running_panel, area, panel_state);
}

pub(super) fn issue_list_title(title: &str, issue_lines: &IssueLines) -> String {
    format!("{title} ({})", issue_lines.rows.len())
}

pub(super) fn issue_line_lines(
    issue_lines: &[IssueLine],
    empty_message: &str,
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

            Line::from(spans)
        })
        .collect()
}

pub(super) fn issue_lines_len(issue_lines: &IssueLines) -> usize {
    if issue_lines.rows.is_empty() {
        1
    } else {
        issue_lines.rows.len()
    }
}

pub(super) fn build_issue_lines(
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
