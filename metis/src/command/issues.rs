use crate::client::MetisClientInterface;
use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use clap::Subcommand;
use metis_common::{
    issues::{
        Issue, IssueDependency, IssueDependencyType, IssueGraphFilter, IssueGraphSelector,
        IssueGraphWildcard, IssueId, IssueRecord, IssueStatus, IssueType, SearchIssuesQuery,
        UpsertIssueRequest,
    },
    patches::{PatchRecord, Review},
    PatchId,
};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    io::{self, Write},
    str::FromStr,
};

#[derive(Debug, Subcommand)]
pub enum IssueCommands {
    /// List Metis issues.
    List {
        /// Filter by issue ID.
        #[arg(long, value_name = "ISSUE_ID", conflicts_with = "query")]
        id: Option<IssueId>,

        /// Pretty-print issues instead of emitting JSONL.
        #[arg(long)]
        pretty: bool,

        /// Filter by issue type.
        #[arg(long, value_name = "ISSUE_TYPE")]
        r#type: Option<IssueType>,

        /// Filter by issue status.
        #[arg(long, value_name = "ISSUE_STATUS")]
        status: Option<IssueStatus>,

        /// Filter by assignee.
        #[arg(long, value_name = "ASSIGNEE")]
        assignee: Option<String>,

        /// Search by query string.
        #[arg(long, value_name = "QUERY")]
        query: Option<String>,

        /// Filter by dependency graph relationships (e.g. '*:child-of:i-abc' or '**:blocked-on:i-def').
        #[arg(
            long = "graph",
            value_name = "FILTER",
            value_parser = parse_issue_graph_filter,
            conflicts_with = "id"
        )]
        graph_filters: Vec<IssueGraphFilter>,
    },
    /// Create a new issue.
    Create {
        /// Issue type: bug, feature, task, chore, or merge-request (defaults to task).
        #[arg(long, value_name = "ISSUE_TYPE", default_value_t = IssueType::Task)]
        r#type: IssueType,

        /// Issue status: open, in-progress, or closed (defaults to open).
        #[arg(long, value_name = "ISSUE_STATUS", default_value_t = IssueStatus::Open)]
        status: IssueStatus,

        /// Issue dependencies in the format dependency-type:ISSUE_ID where dependency-type is child-of or blocked-on (e.g. child-of:i-abcd).
        #[arg(long = "deps", value_name = "TYPE:ISSUE_ID", value_parser = parse_issue_dependency)]
        dependencies: Vec<IssueDependency>,

        /// Patch ids to associate with the issue.
        #[arg(long = "patches", value_name = "PATCH_ID", value_delimiter = ',')]
        patches: Vec<PatchId>,

        /// Assignee for the issue.
        #[arg(long, value_name = "ASSIGNEE")]
        assignee: Option<String>,

        /// Description for the issue.
        #[arg(value_name = "DESCRIPTION")]
        description: String,

        /// Progress notes for the issue.
        #[arg(long, value_name = "PROGRESS")]
        progress: Option<String>,
    },
    /// Update an existing issue.
    Update {
        /// Issue ID to update.
        #[arg(value_name = "ISSUE_ID")]
        id: IssueId,

        /// New issue type.
        #[arg(long, value_name = "ISSUE_TYPE")]
        r#type: Option<IssueType>,

        /// New issue status.
        #[arg(long, value_name = "ISSUE_STATUS")]
        status: Option<IssueStatus>,

        /// Updated assignee.
        #[arg(long, value_name = "ASSIGNEE", conflicts_with = "clear_assignee")]
        assignee: Option<String>,

        /// Remove the current assignee.
        #[arg(long)]
        clear_assignee: bool,

        /// Updated description.
        #[arg(long, value_name = "DESCRIPTION")]
        description: Option<String>,

        /// Replace dependencies with the provided set in the format TYPE:ISSUE_ID (e.g. child-of:i-abcd).
        #[arg(long = "deps", value_name = "TYPE:ISSUE_ID", value_parser = parse_issue_dependency, conflicts_with = "clear_dependencies")]
        dependencies: Vec<IssueDependency>,

        /// Remove all dependencies from the issue.
        #[arg(long)]
        clear_dependencies: bool,

        /// Replace the set of patches associated with the issue.
        #[arg(
            long = "patches",
            value_name = "PATCH_ID",
            value_delimiter = ',',
            conflicts_with = "clear_patches"
        )]
        patches: Vec<PatchId>,

        /// Remove all patches from the issue.
        #[arg(long)]
        clear_patches: bool,

        /// Updated progress notes.
        #[arg(long, value_name = "PROGRESS", conflicts_with = "clear_progress")]
        progress: Option<String>,

        /// Remove all progress notes from the issue.
        #[arg(long)]
        clear_progress: bool,
    },
    /// Describe an issue and its relationships.
    Describe {
        /// Issue ID to describe.
        #[arg(value_name = "ISSUE_ID")]
        id: IssueId,

        /// Pretty-print the description instead of emitting JSON.
        #[arg(long)]
        pretty: bool,
    },
}

pub async fn run(client: &dyn MetisClientInterface, command: IssueCommands) -> Result<()> {
    match command {
        IssueCommands::List {
            id,
            pretty,
            r#type,
            status,
            assignee,
            query,
            graph_filters,
        } => {
            let issues =
                fetch_issues(client, id, r#type, status, assignee, query, graph_filters).await?;
            let mut stdout = io::stdout().lock();
            if pretty {
                print_issues_pretty(&issues, &mut stdout)?;
            } else {
                print_issues_jsonl(&issues, &mut stdout)?;
            }
            Ok(())
        }
        IssueCommands::Create {
            r#type,
            status,
            dependencies,
            patches,
            assignee,
            description,
            progress,
        } => {
            create_issue(
                client,
                r#type,
                status,
                dependencies,
                patches,
                assignee,
                description,
                progress,
            )
            .await
        }
        IssueCommands::Update {
            id,
            r#type,
            status,
            assignee,
            clear_assignee,
            description,
            dependencies,
            clear_dependencies,
            patches,
            clear_patches,
            progress,
            clear_progress,
        } => {
            update_issue(
                client,
                id,
                r#type,
                status,
                assignee,
                clear_assignee,
                description,
                dependencies,
                clear_dependencies,
                patches,
                clear_patches,
                progress,
                clear_progress,
            )
            .await
        }
        IssueCommands::Describe { id, pretty } => describe_issue(client, id, pretty).await,
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct IssueWithPatches {
    issue: IssueRecord,
    patches: Vec<PatchRecord>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct IssueDescription {
    issue: IssueWithPatches,
    parents: Vec<IssueWithPatches>,
    children: Vec<IssueWithPatches>,
}

async fn describe_issue(
    client: &dyn MetisClientInterface,
    id: IssueId,
    pretty: bool,
) -> Result<()> {
    let description = collect_issue_description(client, id).await?;

    let mut stdout = io::stdout().lock();
    if pretty {
        print_issue_description_pretty(&description, &mut stdout)?;
    } else {
        serde_json::to_writer(&mut stdout, &description)?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    }

    Ok(())
}

async fn collect_issue_description(
    client: &dyn MetisClientInterface,
    issue_id: IssueId,
) -> Result<IssueDescription> {
    let issue = client
        .get_issue(&issue_id)
        .await
        .with_context(|| format!("failed to fetch issue '{issue_id}'"))?;

    let parents = fetch_parent_issues(client, &issue).await?;
    let children = fetch_child_issues(client, &issue.id).await?;
    let mut patch_cache = HashMap::new();

    Ok(IssueDescription {
        issue: issue_with_patches(client, issue, &mut patch_cache).await?,
        parents: issues_with_patches(client, parents, &mut patch_cache).await?,
        children: issues_with_patches(client, children, &mut patch_cache).await?,
    })
}

async fn fetch_parent_issues(
    client: &dyn MetisClientInterface,
    issue: &IssueRecord,
) -> Result<Vec<IssueRecord>> {
    let mut parents = Vec::new();
    let mut seen = HashSet::new();

    for dependency in &issue.issue.dependencies {
        if dependency.dependency_type != IssueDependencyType::ChildOf {
            continue;
        }
        if !seen.insert(dependency.issue_id.clone()) {
            continue;
        }

        let parent = client
            .get_issue(&dependency.issue_id)
            .await
            .with_context(|| format!("failed to fetch parent issue '{}'", dependency.issue_id))?;
        parents.push(parent);
    }

    Ok(parents)
}

async fn fetch_child_issues(
    client: &dyn MetisClientInterface,
    issue_id: &IssueId,
) -> Result<Vec<IssueRecord>> {
    let filter = IssueGraphFilter::new(
        IssueGraphSelector::Wildcard(IssueGraphWildcard::Transitive),
        IssueDependencyType::ChildOf,
        IssueGraphSelector::Issue(issue_id.clone()),
    )
    .map_err(|err| anyhow!(err))?;

    let response = client
        .list_issues(&SearchIssuesQuery {
            graph_filters: vec![filter],
            ..SearchIssuesQuery::default()
        })
        .await
        .with_context(|| format!("failed to fetch children for issue '{issue_id}'"))?;

    Ok(response.issues)
}

async fn issues_with_patches(
    client: &dyn MetisClientInterface,
    issues: Vec<IssueRecord>,
    cache: &mut HashMap<PatchId, PatchRecord>,
) -> Result<Vec<IssueWithPatches>> {
    let mut enriched = Vec::with_capacity(issues.len());
    for issue in issues {
        enriched.push(issue_with_patches(client, issue, cache).await?);
    }
    Ok(enriched)
}

async fn issue_with_patches(
    client: &dyn MetisClientInterface,
    issue: IssueRecord,
    cache: &mut HashMap<PatchId, PatchRecord>,
) -> Result<IssueWithPatches> {
    let patches = fetch_patch_records(client, &issue.issue.patches, cache, &issue.id).await?;
    Ok(IssueWithPatches { issue, patches })
}

async fn fetch_patch_records(
    client: &dyn MetisClientInterface,
    patch_ids: &[PatchId],
    cache: &mut HashMap<PatchId, PatchRecord>,
    issue_id: &IssueId,
) -> Result<Vec<PatchRecord>> {
    let mut patches = Vec::with_capacity(patch_ids.len());
    for patch_id in patch_ids {
        if let Some(record) = cache.get(patch_id) {
            patches.push(record.clone());
            continue;
        }

        let record = client.get_patch(patch_id).await.with_context(|| {
            format!("failed to fetch patch '{patch_id}' for issue '{issue_id}'")
        })?;
        cache.insert(patch_id.clone(), record.clone());
        patches.push(record);
    }

    Ok(patches)
}

async fn fetch_issues(
    client: &dyn MetisClientInterface,
    id: Option<IssueId>,
    issue_type: Option<IssueType>,
    status: Option<IssueStatus>,
    assignee: Option<String>,
    query: Option<String>,
    graph_filters: Vec<IssueGraphFilter>,
) -> Result<Vec<IssueRecord>> {
    if let Some(issue_id) = id {
        let record = client
            .get_issue(&issue_id)
            .await
            .with_context(|| format!("failed to fetch issue '{issue_id}'"))?;

        if let Some(expected_type) = issue_type {
            if record.issue.issue_type != expected_type {
                bail!("Issue '{issue_id}' does not match the requested type.");
            }
        }
        if let Some(expected_status) = status {
            if record.issue.status != expected_status {
                bail!("Issue '{issue_id}' does not match the requested status.");
            }
        }
        if let Some(expected_assignee) = assignee {
            let trimmed_assignee = expected_assignee.trim();
            if trimmed_assignee.is_empty() {
                bail!("Assignee filter must not be empty.");
            }
            match record.issue.assignee.as_deref() {
                Some(current) if current.eq_ignore_ascii_case(trimmed_assignee) => {}
                _ => bail!("Issue '{issue_id}' is not assigned to {trimmed_assignee}."),
            }
        }
        return Ok(vec![record]);
    }

    let trimmed_assignee = match assignee {
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                bail!("Assignee filter must not be empty.");
            }
            Some(trimmed.to_string())
        }
        None => None,
    };

    let trimmed_query = query.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    let issues = client
        .list_issues(&SearchIssuesQuery {
            issue_type,
            status,
            assignee: trimmed_assignee.clone(),
            q: trimmed_query,
            graph_filters,
        })
        .await
        .context("failed to list issues")?
        .issues;

    for issue in &issues {
        if let Some(expected_type) = issue_type {
            if issue.issue.issue_type != expected_type {
                bail!("Issue {} does not match the requested type.", issue.id);
            }
        }
        if let Some(expected_status) = status {
            if issue.issue.status != expected_status {
                bail!("Issue {} does not match the requested status.", issue.id);
            }
        }
        if let Some(ref expected_assignee) = trimmed_assignee {
            match issue.issue.assignee.as_deref() {
                Some(current) if current.eq_ignore_ascii_case(expected_assignee) => {}
                _ => bail!("Issue {} is not assigned to {expected_assignee}", issue.id),
            }
        }
    }

    Ok(issues)
}

async fn create_issue(
    client: &dyn MetisClientInterface,
    issue_type: IssueType,
    status: IssueStatus,
    dependencies: Vec<IssueDependency>,
    patches: Vec<PatchId>,
    assignee: Option<String>,
    description: String,
    progress: Option<String>,
) -> Result<()> {
    let description = description.trim();
    if description.is_empty() {
        bail!("Issue description must not be empty.");
    }

    let progress = progress
        .map(|value| value.trim().to_string())
        .unwrap_or_default();

    let assignee = match assignee {
        Some(value) => {
            let trimmed = value.trim().to_string();
            if trimmed.is_empty() {
                bail!("Assignee must not be empty.");
            }
            Some(trimmed)
        }
        None => None,
    };

    let request = UpsertIssueRequest {
        issue: Issue {
            issue_type,
            description: description.to_string(),
            progress,
            status,
            assignee,
            dependencies,
            patches,
        },
        job_id: None,
    };

    let response = client
        .create_issue(&request)
        .await
        .context("failed to create issue")?;

    println!("{}", response.issue_id);
    Ok(())
}

async fn update_issue(
    client: &dyn MetisClientInterface,
    id: IssueId,
    issue_type: Option<IssueType>,
    status: Option<IssueStatus>,
    assignee: Option<String>,
    clear_assignee: bool,
    description: Option<String>,
    dependencies: Vec<IssueDependency>,
    clear_dependencies: bool,
    patches: Vec<PatchId>,
    clear_patches: bool,
    progress: Option<String>,
    clear_progress: bool,
) -> Result<()> {
    let issue_id = id;

    let description = match description {
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                bail!("Issue description must not be empty.");
            }
            Some(trimmed.to_string())
        }
        None => None,
    };

    let assignee = if clear_assignee {
        Some(None)
    } else if let Some(value) = assignee {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            bail!("Assignee must not be empty.");
        }
        Some(Some(trimmed.to_string()))
    } else {
        None
    };

    let dependencies_update = if clear_dependencies {
        Some(Vec::new())
    } else if dependencies.is_empty() {
        None
    } else {
        Some(dependencies)
    };

    let patches_update = if clear_patches {
        Some(Vec::new())
    } else if patches.is_empty() {
        None
    } else {
        Some(patches)
    };

    let progress_update = if clear_progress {
        Some(String::new())
    } else {
        progress.map(|value| value.trim().to_string())
    };

    let no_changes = issue_type.is_none()
        && status.is_none()
        && assignee.is_none()
        && description.is_none()
        && dependencies_update.is_none()
        && patches_update.is_none()
        && progress_update.is_none();
    if no_changes {
        bail!("At least one field must be provided to update.");
    }

    let current = client
        .get_issue(&issue_id)
        .await
        .with_context(|| format!("failed to fetch issue '{issue_id}'"))?;

    let updated_issue = Issue {
        issue_type: issue_type.unwrap_or(current.issue.issue_type),
        description: description.unwrap_or(current.issue.description),
        progress: progress_update.unwrap_or(current.issue.progress),
        status: status.unwrap_or(current.issue.status),
        assignee: assignee.unwrap_or(current.issue.assignee),
        dependencies: dependencies_update.unwrap_or(current.issue.dependencies),
        patches: patches_update.unwrap_or(current.issue.patches),
    };

    let response = client
        .update_issue(
            &issue_id,
            &UpsertIssueRequest {
                issue: updated_issue,
                job_id: None,
            },
        )
        .await
        .with_context(|| format!("failed to update issue '{issue_id}'"))?;

    println!("{}", response.issue_id);
    Ok(())
}

fn parse_issue_graph_filter(raw: &str) -> Result<IssueGraphFilter, String> {
    raw.parse()
}

fn parse_issue_dependency(raw: &str) -> Result<IssueDependency, String> {
    let (dependency_type, issue_id) = raw
        .split_once(':')
        .ok_or_else(|| "dependency must be in the format TYPE:ISSUE_ID".to_string())?;

    let dependency_type =
        IssueDependencyType::from_str(dependency_type).map_err(|err| err.to_string())?;
    let issue_id = issue_id
        .trim()
        .parse::<IssueId>()
        .map_err(|err| err.to_string())?;
    Ok(IssueDependency {
        dependency_type,
        issue_id,
    })
}

fn print_issues_jsonl(issues: &[IssueRecord], writer: &mut impl Write) -> Result<()> {
    for issue in issues {
        serde_json::to_writer(&mut *writer, issue)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn print_issues_pretty(issues: &[IssueRecord], writer: &mut impl Write) -> Result<()> {
    for (index, issue_record) in issues.iter().enumerate() {
        let Issue {
            issue_type,
            description,
            progress,
            status,
            assignee,
            dependencies,
            ..
        } = &issue_record.issue;

        writeln!(writer, "Issue {} ({issue_type}, {status})", issue_record.id)?;
        writeln!(writer, "Assignee: {}", assignee.as_deref().unwrap_or("-"))?;
        writeln!(writer, "Description:")?;
        if description.trim().is_empty() {
            writeln!(writer, "  -")?;
        } else {
            for line in description.lines() {
                writeln!(writer, "  {line}")?;
            }
        }

        writeln!(writer, "Progress:")?;
        if progress.trim().is_empty() {
            writeln!(writer, "  -")?;
        } else {
            for line in progress.lines() {
                writeln!(writer, "  {line}")?;
            }
        }

        if dependencies.is_empty() {
            writeln!(writer, "Dependencies: none")?;
        } else {
            writeln!(writer, "Dependencies:")?;
            for dependency in dependencies {
                writeln!(
                    writer,
                    "  - {} {}",
                    dependency.dependency_type, dependency.issue_id
                )?;
            }
        }

        if index + 1 < issues.len() {
            writeln!(writer)?;
        }
    }
    writer.flush()?;
    Ok(())
}

fn print_issue_description_pretty(
    description: &IssueDescription,
    writer: &mut impl Write,
) -> Result<()> {
    writeln!(writer, "Issue")?;
    write_issue_details_pretty(&description.issue, "  ", writer)?;
    writeln!(writer)?;

    writeln!(writer, "Parents:")?;
    if description.parents.is_empty() {
        writeln!(writer, "  none")?;
    } else {
        for parent in &description.parents {
            write_issue_details_pretty(parent, "  ", writer)?;
            writeln!(writer)?;
        }
    }

    writeln!(writer, "Children (transitive):")?;
    if description.children.is_empty() {
        writeln!(writer, "  none")?;
    } else {
        for child in &description.children {
            write_issue_details_pretty(child, "  ", writer)?;
            writeln!(writer)?;
        }
    }

    writer.flush()?;
    Ok(())
}

fn write_issue_details_pretty(
    issue_with_patches: &IssueWithPatches,
    indent: &str,
    writer: &mut impl Write,
) -> Result<()> {
    let IssueWithPatches {
        issue: issue_record,
        patches: patch_records,
    } = issue_with_patches;
    let Issue {
        issue_type,
        description,
        status,
        assignee,
        dependencies,
        ..
    } = &issue_record.issue;

    writeln!(
        writer,
        "{indent}Issue {} ({issue_type}, {status})",
        issue_record.id
    )?;
    writeln!(
        writer,
        "{indent}Assignee: {}",
        assignee.as_deref().unwrap_or("-")
    )?;
    writeln!(writer, "{indent}Description:")?;
    if description.trim().is_empty() {
        writeln!(writer, "{indent}  -")?;
    } else {
        for line in description.lines() {
            writeln!(writer, "{indent}  {line}")?;
        }
    }

    if dependencies.is_empty() {
        writeln!(writer, "{indent}Dependencies: none")?;
    } else {
        writeln!(writer, "{indent}Dependencies:")?;
        for dependency in dependencies {
            writeln!(
                writer,
                "{indent}  - {} {}",
                dependency.dependency_type, dependency.issue_id
            )?;
        }
    }

    if patch_records.is_empty() {
        writeln!(writer, "{indent}Patches: none")?;
    } else {
        writeln!(writer, "{indent}Patches:")?;
        for patch in patch_records {
            let status = patch.patch.status;
            let title = if patch.patch.title.is_empty() {
                "(untitled)"
            } else {
                patch.patch.title.as_str()
            };
            writeln!(writer, "{indent}  - {title} ({}) [{status}]", patch.id)?;
            writeln!(writer, "{indent}    Description:")?;
            if patch.patch.description.trim().is_empty() {
                writeln!(writer, "{indent}      -")?;
            } else {
                for line in patch.patch.description.lines() {
                    writeln!(writer, "{indent}      {line}")?;
                }
            }
            write_patch_review_summary(&patch.patch.reviews, indent, writer)?;
        }
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PatchReviewSummary {
    approvals: usize,
    change_requests: usize,
    latest_review: ReviewSnapshot,
    reviewer_statuses: Vec<ReviewSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReviewSnapshot {
    author: String,
    is_approved: bool,
    submitted_at: Option<DateTime<Utc>>,
}

fn write_patch_review_summary(
    reviews: &[Review],
    indent: &str,
    writer: &mut impl Write,
) -> Result<()> {
    match build_review_summary(reviews) {
        Some(summary) => {
            writeln!(writer, "{indent}    Reviews:")?;
            writeln!(
                writer,
                "{indent}      Latest: {} by {}{}",
                review_decision(summary.latest_review.is_approved),
                summary.latest_review.author,
                format_timestamp(summary.latest_review.submitted_at.as_ref())
            )?;
            writeln!(
                writer,
                "{indent}      Counts: {} {}, {} {}",
                summary.approvals,
                if summary.approvals == 1 {
                    "approval"
                } else {
                    "approvals"
                },
                summary.change_requests,
                if summary.change_requests == 1 {
                    "change request"
                } else {
                    "change requests"
                }
            )?;
            writeln!(writer, "{indent}      Reviewers:")?;
            for reviewer in summary.reviewer_statuses {
                writeln!(
                    writer,
                    "{indent}        - {}: {}{}",
                    reviewer.author,
                    review_decision(reviewer.is_approved),
                    format_timestamp(reviewer.submitted_at.as_ref())
                )?;
            }
        }
        None => {
            writeln!(writer, "{indent}    Reviews: none")?;
        }
    }

    Ok(())
}

fn build_review_summary(reviews: &[Review]) -> Option<PatchReviewSummary> {
    if reviews.is_empty() {
        return None;
    }

    let mut approvals = 0;
    let mut change_requests = 0;
    let mut latest_review_index: Option<usize> = None;
    let mut latest_by_author = HashMap::new();

    for (index, review) in reviews.iter().enumerate() {
        if review.is_approved {
            approvals += 1;
        } else {
            change_requests += 1;
        }

        match latest_review_index {
            Some(current_index) if !is_more_recent_review(current_index, index, reviews) => {}
            _ => latest_review_index = Some(index),
        }

        match latest_by_author.get(review.author.as_str()) {
            Some(&current_index) if !is_more_recent_review(current_index, index, reviews) => {}
            _ => {
                latest_by_author.insert(review.author.as_str(), index);
            }
        }
    }

    let mut reviewer_statuses: Vec<ReviewSnapshot> = latest_by_author
        .into_iter()
        .map(|(author, index)| {
            let review = &reviews[index];
            ReviewSnapshot {
                author: author.to_string(),
                is_approved: review.is_approved,
                submitted_at: review.submitted_at,
            }
        })
        .collect();
    reviewer_statuses.sort_by(|a, b| a.author.cmp(&b.author));

    let latest_index = latest_review_index.expect("non-empty reviews always set latest index");
    let latest = &reviews[latest_index];

    Some(PatchReviewSummary {
        approvals,
        change_requests,
        latest_review: ReviewSnapshot {
            author: latest.author.clone(),
            is_approved: latest.is_approved,
            submitted_at: latest.submitted_at,
        },
        reviewer_statuses,
    })
}

fn is_more_recent_review(current_index: usize, candidate_index: usize, reviews: &[Review]) -> bool {
    let current = &reviews[current_index];
    let candidate = &reviews[candidate_index];
    match (&current.submitted_at, &candidate.submitted_at) {
        (Some(current_time), Some(candidate_time)) => candidate_time > current_time,
        (None, Some(_)) => true,
        (Some(_), None) => false,
        (None, None) => candidate_index > current_index,
    }
}

fn review_decision(is_approved: bool) -> &'static str {
    if is_approved {
        "approved"
    } else {
        "changes requested"
    }
}

fn format_timestamp(timestamp: Option<&DateTime<Utc>>) -> String {
    timestamp
        .map(|value| value.to_rfc3339_opts(SecondsFormat::Secs, true))
        .map(|value| format!(" @ {value}"))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MockMetisClient;
    use crate::test_utils::ids::{issue_id, patch_id};
    use chrono::{Duration, TimeZone, Utc};
    use metis_common::issues::{
        Issue, IssueGraphSelector, IssueGraphWildcard, IssueRecord, ListIssuesResponse,
        SearchIssuesQuery, UpsertIssueRequest, UpsertIssueResponse,
    };
    use metis_common::patches::{Patch, PatchRecord, Review};

    #[tokio::test]
    async fn list_issues_filters_by_query_and_prints_jsonl() {
        let client = MockMetisClient::default();
        client.push_list_issues_response(ListIssuesResponse {
            issues: vec![IssueRecord {
                id: issue_id("i-1"),
                issue: Issue {
                    issue_type: IssueType::Bug,
                    description: "First issue".into(),
                    progress: String::new(),
                    status: IssueStatus::Open,
                    assignee: None,
                    dependencies: vec![],
                    patches: Vec::new(),
                },
            }],
        });

        let issues = fetch_issues(
            &client,
            None,
            Some(IssueType::Bug),
            Some(IssueStatus::Open),
            None,
            Some("bug".into()),
            Vec::new(),
        )
        .await
        .unwrap();

        assert_eq!(
            client.recorded_list_issue_queries(),
            vec![SearchIssuesQuery {
                issue_type: Some(IssueType::Bug),
                status: Some(IssueStatus::Open),
                assignee: None,
                q: Some("bug".into()),
                graph_filters: Vec::new(),
            }]
        );

        let mut output = Vec::new();
        print_issues_jsonl(&issues, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        let first_id = issue_id("i-1").to_string();
        let second_id = issue_id("i-2").to_string();
        assert!(output.contains(&format!("\"id\":\"{first_id}\"")));
        assert!(!output.contains(&format!("\"id\":\"{second_id}\"")));
    }

    #[tokio::test]
    async fn list_issues_by_id_returns_single_issue() {
        let client = MockMetisClient::default();
        client.push_get_issue_response(IssueRecord {
            id: issue_id("i-123"),
            issue: Issue {
                issue_type: IssueType::Task,
                description: "Edge case bug".into(),
                progress: String::new(),
                status: IssueStatus::InProgress,
                assignee: None,
                dependencies: vec![],
                patches: Vec::new(),
            },
        });

        let issues = fetch_issues(
            &client,
            Some(issue_id("i-123")),
            Some(IssueType::Task),
            Some(IssueStatus::InProgress),
            None,
            None,
            Vec::new(),
        )
        .await
        .unwrap();

        assert_eq!(
            client.recorded_get_issue_requests(),
            vec![issue_id("i-123")]
        );
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].id, issue_id("i-123"));
    }

    #[tokio::test]
    async fn list_issues_filters_by_assignee() {
        let client = MockMetisClient::default();
        client.push_list_issues_response(ListIssuesResponse {
            issues: vec![IssueRecord {
                id: issue_id("i-7"),
                issue: Issue {
                    issue_type: IssueType::Task,
                    description: "Edge case bug".into(),
                    progress: String::new(),
                    status: IssueStatus::Open,
                    assignee: Some("owner-a".into()),
                    dependencies: vec![],
                    patches: Vec::new(),
                },
            }],
        });

        let issues = fetch_issues(
            &client,
            None,
            None,
            None,
            Some("OWNER-A".into()),
            None,
            Vec::new(),
        )
        .await
        .unwrap();

        assert_eq!(
            client.recorded_list_issue_queries(),
            vec![SearchIssuesQuery {
                issue_type: None,
                status: None,
                assignee: Some("OWNER-A".into()),
                q: None,
                graph_filters: Vec::new(),
            }]
        );
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].id, issue_id("i-7"));
    }

    #[tokio::test]
    async fn list_issues_includes_graph_filters_in_query() {
        let client = MockMetisClient::default();
        client.push_list_issues_response(ListIssuesResponse { issues: vec![] });
        let filters = vec![
            parse_issue_graph_filter("*:child-of:i-abcd").unwrap(),
            parse_issue_graph_filter("i-efgh:blocked-on:**").unwrap(),
        ];

        let _ = fetch_issues(&client, None, None, None, None, None, filters.clone())
            .await
            .unwrap();

        assert_eq!(
            client.recorded_list_issue_queries(),
            vec![SearchIssuesQuery {
                issue_type: None,
                status: None,
                assignee: None,
                q: None,
                graph_filters: filters,
            }]
        );
    }

    #[tokio::test]
    async fn describe_issue_collects_related_issues_and_children() {
        let client = MockMetisClient::default();
        let root_id = issue_id("i-root");
        let parent_id = issue_id("i-parent");
        let root_patch_id = patch_id("p-root");
        let parent_patch_id = patch_id("p-parent");
        let child_patch_id = patch_id("p-child");

        let parent_issue = IssueRecord {
            id: parent_id.clone(),
            issue: Issue {
                issue_type: IssueType::Task,
                description: "Parent issue".into(),
                progress: String::new(),
                status: IssueStatus::Open,
                assignee: None,
                dependencies: vec![],
                patches: vec![parent_patch_id.clone()],
            },
        };

        let root_issue = IssueRecord {
            id: root_id.clone(),
            issue: Issue {
                issue_type: IssueType::Task,
                description: "Root issue".into(),
                progress: String::new(),
                status: IssueStatus::Open,
                assignee: Some("owner".into()),
                dependencies: vec![IssueDependency {
                    dependency_type: IssueDependencyType::ChildOf,
                    issue_id: parent_id.clone(),
                }],
                patches: vec![root_patch_id.clone()],
            },
        };

        let child_issue = IssueRecord {
            id: issue_id("i-child"),
            issue: Issue {
                issue_type: IssueType::Bug,
                description: "Child issue".into(),
                progress: String::new(),
                status: IssueStatus::InProgress,
                assignee: None,
                dependencies: vec![IssueDependency {
                    dependency_type: IssueDependencyType::ChildOf,
                    issue_id: root_id.clone(),
                }],
                patches: vec![child_patch_id.clone()],
            },
        };

        client.push_get_issue_response(root_issue.clone());
        client.push_get_issue_response(parent_issue.clone());
        client.push_list_issues_response(ListIssuesResponse {
            issues: vec![child_issue.clone()],
        });
        let root_patch_record = PatchRecord {
            id: root_patch_id.clone(),
            patch: Patch {
                title: "root patch".into(),
                description: "desc".into(),
                diff: "diff".into(),
                status: Default::default(),
                is_automatic_backup: false,
                reviews: Vec::new(),
                service_repo_name: None,
                github: None,
            },
        };
        let parent_patch_record = PatchRecord {
            id: parent_patch_id.clone(),
            patch: Patch {
                title: "parent patch".into(),
                description: "desc".into(),
                diff: "diff".into(),
                status: Default::default(),
                is_automatic_backup: false,
                reviews: Vec::new(),
                service_repo_name: None,
                github: None,
            },
        };
        let child_patch_record = PatchRecord {
            id: child_patch_id.clone(),
            patch: Patch {
                title: "child patch".into(),
                description: "desc".into(),
                diff: "diff".into(),
                status: Default::default(),
                is_automatic_backup: false,
                reviews: Vec::new(),
                service_repo_name: None,
                github: None,
            },
        };
        client.push_get_patch_response(root_patch_record.clone());
        client.push_get_patch_response(parent_patch_record.clone());
        client.push_get_patch_response(child_patch_record.clone());

        let description = collect_issue_description(&client, root_id.clone())
            .await
            .unwrap();

        assert_eq!(
            client.recorded_get_issue_requests(),
            vec![root_id.clone(), parent_id.clone()]
        );
        assert_eq!(
            client.recorded_get_patch_requests(),
            vec![
                root_patch_id.clone(),
                parent_patch_id.clone(),
                child_patch_id.clone()
            ]
        );
        assert_eq!(
            client.recorded_list_issue_queries(),
            vec![SearchIssuesQuery {
                graph_filters: vec![IssueGraphFilter::new(
                    IssueGraphSelector::Wildcard(IssueGraphWildcard::Transitive),
                    IssueDependencyType::ChildOf,
                    IssueGraphSelector::Issue(root_id.clone()),
                )
                .unwrap()],
                ..SearchIssuesQuery::default()
            }]
        );
        assert_eq!(
            description.issue,
            IssueWithPatches {
                issue: root_issue,
                patches: vec![root_patch_record]
            }
        );
        assert_eq!(
            description.parents,
            vec![IssueWithPatches {
                issue: parent_issue,
                patches: vec![parent_patch_record]
            }]
        );
        assert_eq!(
            description.children,
            vec![IssueWithPatches {
                issue: child_issue,
                patches: vec![child_patch_record]
            }]
        );
    }

    #[tokio::test]
    async fn create_issue_submits_issue_record() {
        let client = MockMetisClient::default();
        client.push_upsert_issue_response(UpsertIssueResponse {
            issue_id: issue_id("i-456"),
        });

        let dependencies = vec![IssueDependency {
            dependency_type: IssueDependencyType::ChildOf,
            issue_id: issue_id("i-1"),
        }];
        let patch_ids = vec![patch_id("p-123")];

        create_issue(
            &client,
            IssueType::MergeRequest,
            IssueStatus::Closed,
            dependencies.clone(),
            patch_ids.clone(),
            Some("team-a".into()),
            "New issue description".into(),
            Some("Initial notes".into()),
        )
        .await
        .unwrap();

        assert_eq!(
            client.recorded_issue_upserts(),
            vec![(
                None,
                UpsertIssueRequest {
                    issue: Issue {
                        issue_type: IssueType::MergeRequest,
                        status: IssueStatus::Closed,
                        description: "New issue description".into(),
                        progress: "Initial notes".into(),
                        assignee: Some("team-a".into()),
                        dependencies,
                        patches: patch_ids,
                    },
                    job_id: None,
                }
            )]
        );
    }

    #[tokio::test]
    async fn create_issue_requires_description() {
        let client = MockMetisClient::default();
        assert!(create_issue(
            &client,
            IssueType::Bug,
            IssueStatus::Open,
            vec![],
            Vec::new(),
            None,
            "   ".into(),
            None,
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn create_issue_rejects_empty_assignee() {
        let client = MockMetisClient::default();
        assert!(create_issue(
            &client,
            IssueType::Bug,
            IssueStatus::Open,
            vec![],
            Vec::new(),
            Some("   ".into()),
            "Valid description".into(),
            None,
        )
        .await
        .is_err());
    }

    #[test]
    fn parse_issue_dependency_parses_type_and_id() {
        let dependency = parse_issue_dependency("child-of:i-abcd").unwrap();
        assert_eq!(dependency.dependency_type, IssueDependencyType::ChildOf);
        assert_eq!(dependency.issue_id, issue_id("i-abcd"));
    }

    #[test]
    fn parse_issue_graph_filter_parses_format() {
        let filter = parse_issue_graph_filter("*:child-of:i-abcd").unwrap();
        assert!(matches!(
            filter.lhs,
            IssueGraphSelector::Wildcard(IssueGraphWildcard::Immediate)
        ));
        assert_eq!(filter.literal_issue_id(), &issue_id("i-abcd"));
    }

    #[test]
    fn parse_issue_graph_filter_rejects_invalid_shapes() {
        assert!(parse_issue_graph_filter("i-abcd:child-of:i-efgh").is_err());
    }

    #[tokio::test]
    async fn update_issue_modifies_requested_fields() {
        let client = MockMetisClient::default();
        client.push_get_issue_response(IssueRecord {
            id: issue_id("i-9"),
            issue: Issue {
                issue_type: IssueType::Task,
                description: "Initial issue".into(),
                progress: "Initial note".into(),
                status: IssueStatus::Open,
                assignee: Some("owner-a".into()),
                dependencies: vec![IssueDependency {
                    dependency_type: IssueDependencyType::ChildOf,
                    issue_id: issue_id("i-1"),
                }],
                patches: Vec::new(),
            },
        });
        client.push_upsert_issue_response(UpsertIssueResponse {
            issue_id: issue_id("i-9"),
        });

        update_issue(
            &client,
            issue_id("i-9"),
            Some(IssueType::Bug),
            Some(IssueStatus::Closed),
            Some("owner-b".into()),
            false,
            Some("Updated issue description".into()),
            vec![IssueDependency {
                dependency_type: IssueDependencyType::BlockedOn,
                issue_id: issue_id("i-2"),
            }],
            false,
            vec![patch_id("p-3")],
            false,
            Some("New progress".into()),
            false,
        )
        .await
        .unwrap();

        assert_eq!(client.recorded_get_issue_requests(), vec![issue_id("i-9")]);
        assert_eq!(
            client.recorded_issue_upserts(),
            vec![(
                Some(issue_id("i-9")),
                UpsertIssueRequest {
                    issue: Issue {
                        issue_type: IssueType::Bug,
                        description: "Updated issue description".into(),
                        progress: "New progress".into(),
                        status: IssueStatus::Closed,
                        assignee: Some("owner-b".into()),
                        dependencies: vec![IssueDependency {
                            dependency_type: IssueDependencyType::BlockedOn,
                            issue_id: issue_id("i-2"),
                        }],
                        patches: vec![patch_id("p-3")],
                    },
                    job_id: None,
                }
            )]
        );
    }

    #[tokio::test]
    async fn update_issue_allows_clearing_assignee_and_dependencies() {
        let client = MockMetisClient::default();
        client.push_get_issue_response(IssueRecord {
            id: issue_id("i-10"),
            issue: Issue {
                issue_type: IssueType::Feature,
                description: "Existing issue".into(),
                progress: "Started work".into(),
                status: IssueStatus::InProgress,
                assignee: Some("owner-a".into()),
                dependencies: vec![IssueDependency {
                    dependency_type: IssueDependencyType::BlockedOn,
                    issue_id: issue_id("i-5"),
                }],
                patches: Vec::new(),
            },
        });
        client.push_upsert_issue_response(UpsertIssueResponse {
            issue_id: issue_id("i-10"),
        });

        update_issue(
            &client,
            issue_id("i-10"),
            None,
            None,
            None,
            true,
            None,
            vec![],
            true,
            vec![],
            true,
            None,
            true,
        )
        .await
        .unwrap();

        assert_eq!(
            client.recorded_issue_upserts(),
            vec![(
                Some(issue_id("i-10")),
                UpsertIssueRequest {
                    issue: Issue {
                        issue_type: IssueType::Feature,
                        description: "Existing issue".into(),
                        progress: String::new(),
                        status: IssueStatus::InProgress,
                        assignee: None,
                        dependencies: vec![],
                        patches: Vec::new(),
                    },
                    job_id: None,
                }
            )]
        );
    }

    #[test]
    fn pretty_prints_human_readable_issues() {
        let issues = vec![
            IssueRecord {
                id: issue_id("i-1"),
                issue: Issue {
                    issue_type: IssueType::Bug,
                    description: "First issue\nwith context".into(),
                    progress: "Working on repro".into(),
                    status: IssueStatus::Open,
                    assignee: Some("owner-a".into()),
                    dependencies: vec![IssueDependency {
                        dependency_type: IssueDependencyType::BlockedOn,
                        issue_id: issue_id("i-99"),
                    }],
                    patches: Vec::new(),
                },
            },
            IssueRecord {
                id: issue_id("i-2"),
                issue: Issue {
                    issue_type: IssueType::Feature,
                    description: "Follow-up work".into(),
                    progress: String::new(),
                    status: IssueStatus::InProgress,
                    assignee: None,
                    dependencies: vec![],
                    patches: Vec::new(),
                },
            },
        ];

        let mut output = Vec::new();
        print_issues_pretty(&issues, &mut output).unwrap();
        let rendered = String::from_utf8(output).unwrap();
        let first_issue = issue_id("i-1").to_string();
        let dependency_id = issue_id("i-99").to_string();
        let second_issue = issue_id("i-2").to_string();

        assert!(rendered.contains(&format!("Issue {first_issue} (bug, open)")));
        assert!(rendered.contains("Assignee: owner-a"));
        assert!(rendered.contains("Description:\n  First issue\n  with context"));
        assert!(rendered.contains("Progress:\n  Working on repro"));
        assert!(rendered.contains(&format!("Dependencies:\n  - blocked-on {dependency_id}")));
        assert!(rendered.contains(&format!("Issue {second_issue} (feature, in-progress)")));
        assert!(rendered.contains("Assignee: -"));
        assert!(rendered.contains("Progress:\n  -"));
        assert!(rendered.contains("Dependencies: none"));
        assert!(rendered.contains("Follow-up work"));
    }

    #[test]
    fn describe_issue_pretty_printer_includes_sections() {
        let main_patch_id = patch_id("p-main");
        let main_patch_record = PatchRecord {
            id: main_patch_id.clone(),
            patch: Patch {
                title: "main patch".into(),
                description: "desc".into(),
                diff: "diff".into(),
                status: Default::default(),
                is_automatic_backup: false,
                reviews: Vec::new(),
                service_repo_name: None,
                github: None,
            },
        };
        let description = IssueDescription {
            issue: IssueWithPatches {
                issue: IssueRecord {
                    id: issue_id("i-main"),
                    issue: Issue {
                        issue_type: IssueType::Task,
                        description: "Main issue".into(),
                        progress: String::new(),
                        status: IssueStatus::Open,
                        assignee: Some("owner".into()),
                        dependencies: vec![],
                        patches: vec![main_patch_id],
                    },
                },
                patches: vec![main_patch_record],
            },
            parents: vec![IssueWithPatches {
                issue: IssueRecord {
                    id: issue_id("i-parent"),
                    issue: Issue {
                        issue_type: IssueType::Feature,
                        description: "Parent".into(),
                        progress: String::new(),
                        status: IssueStatus::Open,
                        assignee: None,
                        dependencies: vec![],
                        patches: Vec::new(),
                    },
                },
                patches: Vec::new(),
            }],
            children: vec![],
        };

        let mut output = Vec::new();
        print_issue_description_pretty(&description, &mut output).unwrap();
        let rendered = String::from_utf8(output).unwrap();

        assert!(rendered.contains("Issue"));
        assert!(rendered.contains("Parents:"));
        assert!(rendered.contains("Children (transitive):"));
        assert!(rendered.contains("main patch (p-main) [open]"));
        assert!(rendered.contains("      Description:\n        desc"));
        assert!(rendered.contains("Reviews: none"));
    }

    #[test]
    fn describe_issue_pretty_printer_shows_review_summary() {
        let main_patch_id = patch_id("p-main");
        let earliest_review = Utc.with_ymd_and_hms(2024, 5, 1, 11, 50, 0).unwrap();
        let latest_review = earliest_review + Duration::minutes(10);
        let patch_reviews = vec![
            Review {
                contents: "needs work".to_string(),
                is_approved: false,
                author: "alex".to_string(),
                submitted_at: Some(earliest_review),
            },
            Review {
                contents: "fixed now".to_string(),
                is_approved: false,
                author: "sam".to_string(),
                submitted_at: Some(earliest_review + Duration::minutes(5)),
            },
            Review {
                contents: "ship it".to_string(),
                is_approved: true,
                author: "sam".to_string(),
                submitted_at: Some(latest_review),
            },
        ];
        let description = IssueDescription {
            issue: IssueWithPatches {
                issue: IssueRecord {
                    id: issue_id("i-main"),
                    issue: Issue {
                        issue_type: IssueType::Task,
                        description: "Main issue".into(),
                        progress: String::new(),
                        status: IssueStatus::Open,
                        assignee: Some("owner".into()),
                        dependencies: vec![],
                        patches: vec![main_patch_id.clone()],
                    },
                },
                patches: vec![PatchRecord {
                    id: main_patch_id,
                    patch: Patch {
                        title: "main patch".into(),
                        description: "desc".into(),
                        diff: "diff".into(),
                        status: Default::default(),
                        is_automatic_backup: false,
                        reviews: patch_reviews,
                        service_repo_name: None,
                        github: None,
                    },
                }],
            },
            parents: vec![],
            children: vec![],
        };

        let mut output = Vec::new();
        print_issue_description_pretty(&description, &mut output).unwrap();
        let rendered = String::from_utf8(output).unwrap();

        assert!(
            rendered.contains("Latest: approved by sam @ 2024-05-01T12:00:00Z"),
            "latest review should reflect the newest review"
        );
        assert!(rendered.contains("Counts: 1 approval, 2 change requests"));
        assert!(rendered.contains("Reviewers:"));
        assert!(rendered.contains("- alex: changes requested @ 2024-05-01T11:50:00Z"));
        assert!(rendered.contains("- sam: approved @ 2024-05-01T12:00:00Z"));
    }
}
