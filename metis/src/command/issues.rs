use crate::auth;
use crate::client::MetisClientInterface;
use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use clap::Subcommand;
use metis_common::{
    issues::{
        AddTodoItemRequest, Issue, IssueDependency, IssueDependencyType, IssueGraphFilter,
        IssueGraphSelector, IssueGraphWildcard, IssueId, IssueRecord, IssueStatus, IssueType,
        JobSettings, ReplaceTodoListRequest, SearchIssuesQuery, SetTodoItemStatusRequest, TodoItem,
        UpsertIssueRequest,
    },
    patches::{PatchRecord, Review},
    users::{ResolveUserRequest, User},
    PatchId, RepoName,
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

        /// Creator for the issue (must match the authenticated user).
        #[arg(long, value_name = "CREATOR")]
        creator: Option<String>,

        /// Description for the issue.
        #[arg(value_name = "DESCRIPTION")]
        description: String,

        /// Progress notes for the issue.
        #[arg(long, value_name = "PROGRESS")]
        progress: Option<String>,

        /// Repository name to use for job settings.
        #[arg(long = "repo-name", value_name = "REPO_NAME")]
        repo_name: Option<String>,

        /// Git remote URL to use for job settings.
        #[arg(long = "remote-url", value_name = "REMOTE_URL")]
        remote_url: Option<String>,

        /// Container image to use for job settings.
        #[arg(long, value_name = "IMAGE")]
        image: Option<String>,

        /// Branch to use for job settings.
        #[arg(long, value_name = "BRANCH")]
        branch: Option<String>,

        /// Maximum retries to use for job settings.
        #[arg(long = "max-retries", value_name = "MAX_RETRIES")]
        max_retries: Option<u32>,
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

        /// Updated creator (must match the authenticated user).
        #[arg(long, value_name = "CREATOR")]
        creator: Option<String>,

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

        /// Repository name to use for job settings.
        #[arg(
            long = "repo-name",
            value_name = "REPO_NAME",
            conflicts_with = "clear_job_settings"
        )]
        repo_name: Option<String>,

        /// Git remote URL to use for job settings.
        #[arg(
            long = "remote-url",
            value_name = "REMOTE_URL",
            conflicts_with = "clear_job_settings"
        )]
        remote_url: Option<String>,

        /// Container image to use for job settings.
        #[arg(long, value_name = "IMAGE", conflicts_with = "clear_job_settings")]
        image: Option<String>,

        /// Branch to use for job settings.
        #[arg(long, value_name = "BRANCH", conflicts_with = "clear_job_settings")]
        branch: Option<String>,

        /// Maximum retries to use for job settings.
        #[arg(
            long = "max-retries",
            value_name = "MAX_RETRIES",
            conflicts_with = "clear_job_settings"
        )]
        max_retries: Option<u32>,

        /// Remove all job settings from the issue.
        #[arg(long)]
        clear_job_settings: bool,
    },
    /// Inspect or update an issue's todo list.
    Todo {
        /// Issue ID to operate on.
        #[arg(value_name = "ISSUE_ID")]
        id: IssueId,

        /// Append a new todo item (prefix with '[x]' to mark done immediately).
        #[arg(long, value_name = "TEXT", conflicts_with_all = ["done", "undone", "replace"])]
        add: Option<String>,

        /// Mark a todo item as done (1-indexed).
        #[arg(
            long,
            value_name = "ITEM_NUMBER",
            value_parser = clap::value_parser!(usize),
            conflicts_with_all = ["add", "undone", "replace"]
        )]
        done: Option<usize>,

        /// Mark a todo item as not done (1-indexed).
        #[arg(
            long,
            value_name = "ITEM_NUMBER",
            value_parser = clap::value_parser!(usize),
            conflicts_with_all = ["add", "done", "replace"]
        )]
        undone: Option<usize>,

        /// Replace the entire todo list with the provided ordered items.
        #[arg(
            long,
            value_name = "ITEM",
            num_args = 1..,
            value_delimiter = ',',
            conflicts_with_all = ["add", "done", "undone"]
        )]
        replace: Option<Vec<String>>,
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
            let mut buffer = Vec::new();
            if pretty {
                print_issues_pretty(&issues, &mut buffer)?;
            } else {
                print_issues_jsonl(&issues, &mut buffer)?;
            }
            io::stdout().write_all(&buffer)?;
            io::stdout().flush()?;
            Ok(())
        }
        IssueCommands::Create {
            r#type,
            status,
            dependencies,
            patches,
            assignee,
            creator,
            description,
            progress,
            repo_name,
            remote_url,
            image,
            branch,
            max_retries,
        } => {
            create_issue(
                client,
                r#type,
                status,
                dependencies,
                patches,
                assignee,
                creator,
                description,
                progress,
                repo_name,
                remote_url,
                image,
                branch,
                max_retries,
            )
            .await
        }
        IssueCommands::Update {
            id,
            r#type,
            status,
            assignee,
            clear_assignee,
            creator,
            description,
            dependencies,
            clear_dependencies,
            patches,
            clear_patches,
            progress,
            clear_progress,
            repo_name,
            remote_url,
            image,
            branch,
            max_retries,
            clear_job_settings,
        } => {
            update_issue(
                client,
                id,
                r#type,
                status,
                assignee,
                clear_assignee,
                creator,
                description,
                dependencies,
                clear_dependencies,
                patches,
                clear_patches,
                progress,
                clear_progress,
                repo_name,
                remote_url,
                image,
                branch,
                max_retries,
                clear_job_settings,
            )
            .await
        }
        IssueCommands::Todo {
            id,
            add,
            done,
            undone,
            replace,
        } => manage_todo_list(client, id, add, done, undone, replace).await,
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

    let mut buffer = Vec::new();
    if pretty {
        print_issue_description_pretty(&description, &mut buffer)?;
    } else {
        serde_json::to_writer(&mut buffer, &description)?;
        buffer.write_all(b"\n")?;
    }
    io::stdout().write_all(&buffer)?;
    io::stdout().flush()?;

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
        .list_issues(&SearchIssuesQuery::new(
            None,
            None,
            None,
            None,
            vec![filter],
        ))
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
        .list_issues(&SearchIssuesQuery::new(
            issue_type,
            status,
            trimmed_assignee.clone(),
            trimmed_query,
            graph_filters,
        ))
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

fn resolve_job_settings(
    current: Option<JobSettings>,
    repo_name: Option<String>,
    remote_url: Option<String>,
    image: Option<String>,
    branch: Option<String>,
    max_retries: Option<u32>,
    clear_job_settings: bool,
) -> Result<(Option<JobSettings>, bool)> {
    if clear_job_settings {
        return Ok((None, true));
    }

    let mut changed = false;
    let mut job_settings = current.clone().unwrap_or_default();

    if let Some(value) = repo_name {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            bail!("--repo-name must not be empty when provided.");
        }
        let parsed = RepoName::from_str(trimmed)
            .map_err(|err| anyhow!("invalid repo name '{trimmed}': {err}"))?;
        job_settings.repo_name = Some(parsed);
        changed = true;
    }

    if let Some(value) = remote_url {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            bail!("--remote-url must not be empty when provided.");
        }
        job_settings.remote_url = Some(trimmed.to_string());
        changed = true;
    }

    if let Some(value) = image {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            bail!("--image must not be empty when provided.");
        }
        job_settings.image = Some(trimmed.to_string());
        changed = true;
    }

    if let Some(value) = branch {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            bail!("--branch must not be empty when provided.");
        }
        job_settings.branch = Some(trimmed.to_string());
        changed = true;
    }

    if let Some(value) = max_retries {
        job_settings.max_retries = Some(value);
        changed = true;
    }

    if changed {
        Ok((Some(job_settings), true))
    } else {
        Ok((current, false))
    }
}

async fn create_issue(
    client: &dyn MetisClientInterface,
    issue_type: IssueType,
    status: IssueStatus,
    dependencies: Vec<IssueDependency>,
    patches: Vec<PatchId>,
    assignee: Option<String>,
    creator: Option<String>,
    description: String,
    progress: Option<String>,
    repo_name: Option<String>,
    remote_url: Option<String>,
    image: Option<String>,
    branch: Option<String>,
    max_retries: Option<u32>,
) -> Result<()> {
    let description = description.trim();
    if description.is_empty() {
        bail!("Issue description must not be empty.");
    }

    let progress = progress
        .map(|value| value.trim().to_string())
        .unwrap_or_default();

    let creator = match creator {
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                bail!("Creator must not be empty.");
            }
            let authenticated_user = resolve_authenticated_user(client).await?;
            if !creator_matches_user(trimmed, &authenticated_user) {
                bail!(
                    "Creator override must match the authenticated user ({})",
                    authenticated_user.username.as_ref()
                );
            }
            authenticated_user
        }
        None => {
            if let Some(parent_id) = dependencies
                .iter()
                .find(|dependency| dependency.dependency_type == IssueDependencyType::ChildOf)
                .map(|dependency| dependency.issue_id.clone())
            {
                let parent = client
                    .get_issue(&parent_id)
                    .await
                    .with_context(|| format!("failed to fetch parent issue '{parent_id}'"))?;
                parent.issue.creator
            } else {
                resolve_authenticated_user(client).await?
            }
        }
    };

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

    let (job_settings, _) = resolve_job_settings(
        None,
        repo_name,
        remote_url,
        image,
        branch,
        max_retries,
        false,
    )?;

    let request = UpsertIssueRequest::new(
        Issue::new(
            issue_type,
            description.to_string(),
            creator,
            progress,
            status,
            assignee,
            job_settings,
            Vec::new(),
            dependencies,
            patches,
        ),
        None,
    );

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
    creator: Option<String>,
    description: Option<String>,
    dependencies: Vec<IssueDependency>,
    clear_dependencies: bool,
    patches: Vec<PatchId>,
    clear_patches: bool,
    progress: Option<String>,
    clear_progress: bool,
    repo_name: Option<String>,
    remote_url: Option<String>,
    image: Option<String>,
    branch: Option<String>,
    max_retries: Option<u32>,
    clear_job_settings: bool,
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

    let creator = if let Some(value) = creator {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            bail!("Creator must not be empty.");
        }
        let authenticated_user = resolve_authenticated_user(client).await?;
        if !creator_matches_user(trimmed, &authenticated_user) {
            bail!(
                "Creator override must match the authenticated user ({})",
                authenticated_user.username.as_ref()
            );
        }
        Some(authenticated_user)
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

    let job_settings_requested = clear_job_settings
        || repo_name.is_some()
        || remote_url.is_some()
        || image.is_some()
        || branch.is_some()
        || max_retries.is_some();

    let no_changes = issue_type.is_none()
        && status.is_none()
        && assignee.is_none()
        && creator.is_none()
        && description.is_none()
        && dependencies_update.is_none()
        && patches_update.is_none()
        && progress_update.is_none()
        && !job_settings_requested;
    if no_changes {
        bail!("At least one field must be provided to update.");
    }

    let current = client
        .get_issue(&issue_id)
        .await
        .with_context(|| format!("failed to fetch issue '{issue_id}'"))?;

    let (job_settings, _) = resolve_job_settings(
        current.issue.job_settings.clone(),
        repo_name,
        remote_url,
        image,
        branch,
        max_retries,
        clear_job_settings,
    )?;

    let updated_issue = Issue::new(
        issue_type.unwrap_or(current.issue.issue_type),
        description.unwrap_or(current.issue.description),
        creator.unwrap_or(current.issue.creator),
        progress_update.unwrap_or(current.issue.progress),
        status.unwrap_or(current.issue.status),
        assignee.unwrap_or(current.issue.assignee),
        job_settings,
        current.issue.todo_list,
        dependencies_update.unwrap_or(current.issue.dependencies),
        patches_update.unwrap_or(current.issue.patches),
    );

    let response = client
        .update_issue(&issue_id, &UpsertIssueRequest::new(updated_issue, None))
        .await
        .with_context(|| format!("failed to update issue '{issue_id}'"))?;

    println!("{}", response.issue_id);
    Ok(())
}

async fn resolve_authenticated_user(client: &dyn MetisClientInterface) -> Result<User> {
    let token = auth::ensure_auth_token(client).await?;
    let response = client
        .resolve_user(&ResolveUserRequest::new(token.clone()))
        .await
        .context("failed to resolve user from auth token")?;
    let mut user = User::new(response.user.username, token);
    user.github_user_id = response.user.github_user_id;
    Ok(user)
}

fn creator_matches_user(creator: &str, user: &User) -> bool {
    user.username.as_ref().eq_ignore_ascii_case(creator)
}

async fn manage_todo_list(
    client: &dyn MetisClientInterface,
    issue_id: IssueId,
    add: Option<String>,
    done: Option<usize>,
    undone: Option<usize>,
    replace: Option<Vec<String>>,
) -> Result<()> {
    let todo_list = resolve_todo_list(client, &issue_id, add, done, undone, replace).await?;
    print_todo_list(&issue_id, &todo_list)?;
    Ok(())
}

async fn resolve_todo_list(
    client: &dyn MetisClientInterface,
    issue_id: &IssueId,
    add: Option<String>,
    done: Option<usize>,
    undone: Option<usize>,
    replace: Option<Vec<String>>,
) -> Result<Vec<TodoItem>> {
    if let Some(items) = replace {
        let todo_list = parse_todo_items(items)?;
        let response = client
            .replace_todo_list(issue_id, &ReplaceTodoListRequest::new(todo_list))
            .await
            .with_context(|| format!("failed to replace todo list for issue '{issue_id}'"))?;
        return Ok(response.todo_list);
    }

    if let Some(text) = add {
        let item = parse_todo_item_input(&text)?;
        let response = client
            .add_todo_item(
                issue_id,
                &AddTodoItemRequest::new(item.description, item.is_done),
            )
            .await
            .with_context(|| format!("failed to add todo item for issue '{issue_id}'"))?;
        return Ok(response.todo_list);
    }

    if let Some(item_number) = done {
        let item_number = validate_item_number(item_number)?;
        let response = client
            .set_todo_item_status(issue_id, item_number, &SetTodoItemStatusRequest::new(true))
            .await
            .with_context(|| {
                format!("failed to mark todo item {item_number} done for issue '{issue_id}'")
            })?;
        return Ok(response.todo_list);
    }

    if let Some(item_number) = undone {
        let item_number = validate_item_number(item_number)?;
        let response = client
            .set_todo_item_status(issue_id, item_number, &SetTodoItemStatusRequest::new(false))
            .await
            .with_context(|| {
                format!("failed to mark todo item {item_number} undone for issue '{issue_id}'")
            })?;
        return Ok(response.todo_list);
    }

    let issue = client
        .get_issue(issue_id)
        .await
        .with_context(|| format!("failed to fetch issue '{issue_id}'"))?;
    Ok(issue.issue.todo_list)
}

fn parse_todo_items(raw_items: Vec<String>) -> Result<Vec<TodoItem>> {
    raw_items
        .into_iter()
        .map(|value| parse_todo_item_input(&value))
        .collect()
}

fn validate_item_number(item_number: usize) -> Result<usize> {
    if item_number == 0 {
        bail!("Todo item number must be at least 1.");
    }
    Ok(item_number)
}

fn parse_todo_item_input(raw: &str) -> Result<TodoItem> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("Todo item description must not be empty.");
    }

    let (is_done, description) = if let Some(rest) = trimmed.strip_prefix("[x]") {
        (true, rest)
    } else if let Some(rest) = trimmed.strip_prefix("[X]") {
        (true, rest)
    } else if let Some(rest) = trimmed.strip_prefix("[ ]") {
        (false, rest)
    } else {
        (false, trimmed)
    };

    let description = description.trim().to_string();
    if description.is_empty() {
        bail!("Todo item description must not be empty.");
    }

    Ok(TodoItem::new(description, is_done))
}

fn print_todo_list(issue_id: &IssueId, todo_list: &[TodoItem]) -> Result<()> {
    let mut buffer = Vec::new();
    render_todo_list(issue_id, todo_list, &mut buffer)?;
    io::stdout().write_all(&buffer)?;
    io::stdout().flush()?;
    Ok(())
}

fn render_todo_list(
    issue_id: &IssueId,
    todo_list: &[TodoItem],
    writer: &mut impl Write,
) -> Result<()> {
    writeln!(writer, "Todos for issue {issue_id}:")?;
    if todo_list.is_empty() {
        writeln!(writer, "  none")?;
        writer.flush()?;
        return Ok(());
    }

    for (index, item) in todo_list.iter().enumerate() {
        let status = if item.is_done { "[x]" } else { "[ ]" };
        let prefix = format!("  {}. {status} ", index + 1);
        let continuation_indent = " ".repeat(prefix.len());
        let mut lines = item.description.lines();

        if let Some(first_line) = lines.next() {
            writeln!(writer, "{prefix}{first_line}")?;
        } else {
            writeln!(writer, "{prefix}-")?;
        }

        for line in lines {
            writeln!(writer, "{continuation_indent}{line}")?;
        }
    }

    writer.flush()?;
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
    Ok(IssueDependency::new(dependency_type, issue_id))
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
            creator,
            progress,
            status,
            assignee,
            dependencies,
            ..
        } = &issue_record.issue;

        writeln!(writer, "Issue {} ({issue_type}, {status})", issue_record.id)?;
        writeln!(writer, "Creator: {}", creator.username.as_ref())?;
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
    write_issue_details_pretty(&description.issue, "  ", true, writer)?;
    writeln!(writer)?;

    writeln!(writer, "Parents:")?;
    if description.parents.is_empty() {
        writeln!(writer, "  none")?;
    } else {
        for parent in &description.parents {
            write_issue_details_pretty(parent, "  ", false, writer)?;
            writeln!(writer)?;
        }
    }

    writeln!(writer, "Children (transitive):")?;
    if description.children.is_empty() {
        writeln!(writer, "  none")?;
    } else {
        for child in &description.children {
            write_issue_details_pretty(child, "  ", false, writer)?;
            writeln!(writer)?;
        }
    }

    writer.flush()?;
    Ok(())
}

fn write_issue_details_pretty(
    issue_with_patches: &IssueWithPatches,
    indent: &str,
    show_todo_list: bool,
    writer: &mut impl Write,
) -> Result<()> {
    let IssueWithPatches {
        issue: issue_record,
        patches: patch_records,
    } = issue_with_patches;
    let Issue {
        issue_type,
        description,
        creator,
        progress,
        status,
        assignee,
        dependencies,
        todo_list,
        ..
    } = &issue_record.issue;

    writeln!(
        writer,
        "{indent}Issue {} ({issue_type}, {status})",
        issue_record.id
    )?;
    writeln!(writer, "{indent}Creator: {}", creator.username.as_ref())?;
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

    writeln!(writer, "{indent}Progress:")?;
    if progress.trim().is_empty() {
        writeln!(writer, "{indent}  -")?;
    } else {
        for line in progress.lines() {
            writeln!(writer, "{indent}  {line}")?;
        }
    }

    if show_todo_list {
        write_todo_list(indent, todo_list, writer)?;
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

fn write_todo_list(indent: &str, todo_list: &[TodoItem], writer: &mut impl Write) -> Result<()> {
    writeln!(writer, "{indent}Todos:")?;
    if todo_list.is_empty() {
        writeln!(writer, "{indent}  none")?;
        return Ok(());
    }

    for (index, item) in todo_list.iter().enumerate() {
        let status = if item.is_done { "[x]" } else { "[ ]" };
        let prefix = format!("{indent}  {}. {status} ", index + 1);
        let continuation_indent = " ".repeat(prefix.len());
        let mut lines = item.description.lines();

        if let Some(first_line) = lines.next() {
            writeln!(writer, "{prefix}{first_line}")?;
        } else {
            writeln!(writer, "{prefix}-")?;
        }

        for line in lines {
            writeln!(writer, "{continuation_indent}{line}")?;
        }
    }

    Ok(())
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
    use crate::client::MetisClient;
    use crate::test_utils::env as test_env;
    use crate::test_utils::ids::{issue_id, patch_id};
    use chrono::{Duration, TimeZone, Utc};
    use httpmock::prelude::*;
    use metis_common::issues::{
        AddTodoItemRequest, Issue, IssueGraphSelector, IssueGraphWildcard, IssueRecord,
        JobSettings, ListIssuesResponse, ReplaceTodoListRequest, SetTodoItemStatusRequest,
        TodoItem, TodoListResponse, UpsertIssueRequest, UpsertIssueResponse,
    };
    use metis_common::{
        patches::{Patch, PatchRecord, Review},
        users::{ResolveUserRequest, ResolveUserResponse, User, UserSummary, Username},
        PatchId, RepoName,
    };
    use reqwest::Client as HttpClient;
    use std::env;
    use std::fs;
    use std::str::FromStr;
    use tempfile::tempdir;

    fn sample_diff() -> String {
        "--- a/file.txt\n+++ b/file.txt\n@@\n-old\n+new\n".to_string()
    }

    fn sample_repo_name() -> RepoName {
        RepoName::from_str("dourolabs/example").unwrap()
    }

    fn sample_job_settings() -> JobSettings {
        let mut job_settings = JobSettings::default();
        job_settings.repo_name = Some(sample_repo_name());
        job_settings.remote_url = Some("https://example.com/service.git".into());
        job_settings.image = Some("worker:123".into());
        job_settings.branch = Some("main".into());
        job_settings.max_retries = Some(5);
        job_settings
    }

    fn user(username: &str) -> User {
        User::new(Username::from(username), String::new())
    }

    fn empty_user() -> User {
        User::new(Username::from(""), String::new())
    }

    fn metis_client(server: &MockServer) -> MetisClient {
        MetisClient::with_http_client(server.base_url(), HttpClient::new()).unwrap()
    }

    fn api_issue_record(
        id: &str,
        issue_type: IssueType,
        description: &str,
        status: IssueStatus,
        assignee: Option<&str>,
        dependencies: Vec<IssueDependency>,
        patches: Vec<PatchId>,
    ) -> IssueRecord {
        IssueRecord::new(
            issue_id(id),
            Issue::new(
                issue_type,
                description.into(),
                empty_user(),
                String::new(),
                status,
                assignee.map(str::to_string),
                None,
                Vec::new(),
                dependencies,
                patches,
            ),
        )
    }

    #[tokio::test]
    async fn list_issues_filters_by_query_and_prints_jsonl() {
        let server = MockServer::start();
        let client = metis_client(&server);
        let issues_response = ListIssuesResponse::new(vec![IssueRecord::new(
            issue_id("i-1"),
            Issue::new(
                IssueType::Bug,
                "First issue".into(),
                empty_user(),
                String::new(),
                IssueStatus::Open,
                None,
                None,
                Vec::new(),
                vec![],
                Vec::new(),
            ),
        )]);
        let list_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/issues")
                .query_param("issue_type", IssueType::Bug.as_str())
                .query_param("status", IssueStatus::Open.as_str())
                .query_param("q", "bug");
            then.status(200).json_body_obj(&issues_response);
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

        list_mock.assert();
        assert_eq!(list_mock.hits(), 1);

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
        let server = MockServer::start();
        let client = metis_client(&server);
        let issue_id = issue_id("i-123");
        let issue_record = IssueRecord::new(
            issue_id.clone(),
            Issue::new(
                IssueType::Task,
                "Edge case bug".into(),
                empty_user(),
                String::new(),
                IssueStatus::InProgress,
                None,
                None,
                Vec::new(),
                vec![],
                Vec::new(),
            ),
        );
        let get_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{issue_id}").as_str());
            then.status(200).json_body_obj(&issue_record);
        });

        let issues = fetch_issues(
            &client,
            Some(issue_id.clone()),
            Some(IssueType::Task),
            Some(IssueStatus::InProgress),
            None,
            None,
            Vec::new(),
        )
        .await
        .unwrap();

        get_mock.assert();
        assert_eq!(get_mock.hits(), 1);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].id, issue_id);
    }

    #[tokio::test]
    async fn list_issues_filters_by_assignee() {
        let server = MockServer::start();
        let client = metis_client(&server);
        let issues_response = ListIssuesResponse::new(vec![IssueRecord::new(
            issue_id("i-7"),
            Issue::new(
                IssueType::Task,
                "Edge case bug".into(),
                empty_user(),
                String::new(),
                IssueStatus::Open,
                Some("owner-a".into()),
                None,
                Vec::new(),
                vec![],
                Vec::new(),
            ),
        )]);
        let list_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/issues")
                .query_param("assignee", "OWNER-A");
            then.status(200).json_body_obj(&issues_response);
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

        list_mock.assert();
        assert_eq!(list_mock.hits(), 1);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].id, issue_id("i-7"));
    }

    #[tokio::test]
    async fn list_issues_includes_graph_filters_in_query() {
        let server = MockServer::start();
        let client = metis_client(&server);
        let filters = vec![
            parse_issue_graph_filter("*:child-of:i-abcd").unwrap(),
            parse_issue_graph_filter("i-efgh:blocked-on:**").unwrap(),
        ];
        let graph_query = filters
            .iter()
            .map(|filter| filter.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let list_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/issues")
                .query_param("graph", graph_query.as_str());
            then.status(200)
                .json_body_obj(&ListIssuesResponse::new(vec![]));
        });

        let _ = fetch_issues(&client, None, None, None, None, None, filters.clone())
            .await
            .unwrap();

        list_mock.assert();
        assert_eq!(list_mock.hits(), 1);
    }

    #[tokio::test]
    async fn describe_issue_collects_related_issues_and_children() {
        let server = MockServer::start();
        let client = metis_client(&server);
        let root_id = issue_id("i-root");
        let parent_id = issue_id("i-parent");
        let root_patch_id = patch_id("p-root");
        let parent_patch_id = patch_id("p-parent");
        let child_patch_id = patch_id("p-child");

        let parent_issue = api_issue_record(
            "i-parent",
            IssueType::Task,
            "Parent issue",
            IssueStatus::Open,
            None,
            vec![],
            vec![parent_patch_id.clone()],
        );

        let root_issue = api_issue_record(
            "i-root",
            IssueType::Task,
            "Root issue",
            IssueStatus::Open,
            Some("owner"),
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
            vec![root_patch_id.clone()],
        );

        let child_issue = api_issue_record(
            "i-child",
            IssueType::Bug,
            "Child issue",
            IssueStatus::InProgress,
            None,
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                root_id.clone(),
            )],
            vec![child_patch_id.clone()],
        );

        let root_issue_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{root_id}").as_str());
            then.status(200).json_body_obj(&root_issue);
        });
        let parent_issue_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{parent_id}").as_str());
            then.status(200).json_body_obj(&parent_issue);
        });
        let graph_query = IssueGraphFilter::new(
            IssueGraphSelector::Wildcard(IssueGraphWildcard::Transitive),
            IssueDependencyType::ChildOf,
            IssueGraphSelector::Issue(root_id.clone()),
        )
        .unwrap()
        .to_string();
        let list_children_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/issues")
                .query_param("graph", graph_query.as_str());
            then.status(200)
                .json_body_obj(&ListIssuesResponse::new(vec![child_issue.clone()]));
        });
        let root_patch_record = PatchRecord::new(
            root_patch_id.clone(),
            Patch::new(
                "root patch".into(),
                "desc".into(),
                sample_diff(),
                Default::default(),
                false,
                None,
                Vec::new(),
                sample_repo_name(),
                None,
            ),
        );
        let parent_patch_record = PatchRecord::new(
            parent_patch_id.clone(),
            Patch::new(
                "parent patch".into(),
                "desc".into(),
                sample_diff(),
                Default::default(),
                false,
                None,
                Vec::new(),
                sample_repo_name(),
                None,
            ),
        );
        let child_patch_record = PatchRecord::new(
            child_patch_id.clone(),
            Patch::new(
                "child patch".into(),
                "desc".into(),
                sample_diff(),
                Default::default(),
                false,
                None,
                Vec::new(),
                sample_repo_name(),
                None,
            ),
        );
        let root_patch_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/patches/{root_patch_id}").as_str());
            then.status(200).json_body_obj(&root_patch_record);
        });
        let parent_patch_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/patches/{parent_patch_id}").as_str());
            then.status(200).json_body_obj(&parent_patch_record);
        });
        let child_patch_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/patches/{child_patch_id}").as_str());
            then.status(200).json_body_obj(&child_patch_record);
        });

        let description = collect_issue_description(&client, root_id.clone())
            .await
            .unwrap();

        root_issue_mock.assert();
        parent_issue_mock.assert();
        list_children_mock.assert();
        root_patch_mock.assert();
        parent_patch_mock.assert();
        child_patch_mock.assert();
        assert_eq!(root_issue_mock.hits(), 1);
        assert_eq!(parent_issue_mock.hits(), 1);
        assert_eq!(list_children_mock.hits(), 1);
        assert_eq!(root_patch_mock.hits(), 1);
        assert_eq!(parent_patch_mock.hits(), 1);
        assert_eq!(child_patch_mock.hits(), 1);
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
    #[allow(clippy::await_holding_lock)]
    async fn create_issue_submits_issue_record() {
        let _guard = test_env::lock();
        let server = MockServer::start();
        let client = metis_client(&server);
        let original_home = env::var_os("HOME");
        let temp = tempdir().expect("tempdir");
        env::set_var("HOME", temp.path());
        let auth_token_path = temp.path().join(".local/share/metis/auth-token");
        fs::create_dir_all(auth_token_path.parent().expect("auth token parent"))
            .expect("create auth token dir");
        fs::write(&auth_token_path, "token-123").expect("write auth token");

        let patch_ids = vec![patch_id("p-123")];
        let resolve_request = ResolveUserRequest::new("token-123".into());
        let resolve_response =
            ResolveUserResponse::new(UserSummary::new(Username::from("creator-a")));
        let resolve_mock = server.mock(|when, then| {
            when.method(POST)
                .path("/v1/users/resolve")
                .json_body_obj(&resolve_request);
            then.status(200).json_body_obj(&resolve_response);
        });
        let create_request = UpsertIssueRequest::new(
            Issue::new(
                IssueType::MergeRequest,
                "New issue description".into(),
                {
                    let mut creator = User::new(Username::from("creator-a"), "token-123".into());
                    creator.github_user_id = resolve_response.user.github_user_id;
                    creator
                },
                "Initial notes".into(),
                IssueStatus::Closed,
                Some("team-a".into()),
                None,
                Vec::new(),
                Vec::new(),
                patch_ids.clone(),
            ),
            None,
        );
        let create_mock = server.mock(|when, then| {
            when.method(POST)
                .path("/v1/issues")
                .json_body_obj(&create_request);
            then.status(200)
                .json_body_obj(&UpsertIssueResponse::new(issue_id("i-456")));
        });

        create_issue(
            &client,
            IssueType::MergeRequest,
            IssueStatus::Closed,
            Vec::new(),
            patch_ids.clone(),
            Some("team-a".into()),
            None,
            "New issue description".into(),
            Some("Initial notes".into()),
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        resolve_mock.assert();
        create_mock.assert();
        assert_eq!(resolve_mock.hits(), 1);
        assert_eq!(create_mock.hits(), 1);
        match original_home {
            Some(value) => env::set_var("HOME", value),
            None => env::remove_var("HOME"),
        }
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn create_issue_sets_job_settings() {
        let _guard = test_env::lock();
        let server = MockServer::start();
        let client = metis_client(&server);
        let original_home = env::var_os("HOME");
        let temp = tempdir().expect("tempdir");
        env::set_var("HOME", temp.path());
        let auth_token_path = temp.path().join(".local/share/metis/auth-token");
        fs::create_dir_all(auth_token_path.parent().expect("auth token parent"))
            .expect("create auth token dir");
        fs::write(&auth_token_path, "token-123").expect("write auth token");

        let mut job_settings = sample_job_settings();
        job_settings.image = Some("worker:latest".into());
        job_settings.branch = Some("feature/job-settings".into());
        job_settings.max_retries = Some(4);
        let resolve_request = ResolveUserRequest::new("token-123".into());
        let resolve_response =
            ResolveUserResponse::new(UserSummary::new(Username::from("creator-a")));
        let resolve_mock = server.mock(|when, then| {
            when.method(POST)
                .path("/v1/users/resolve")
                .json_body_obj(&resolve_request);
            then.status(200).json_body_obj(&resolve_response);
        });
        let create_request = UpsertIssueRequest::new(
            Issue::new(
                IssueType::MergeRequest,
                "New issue description".into(),
                {
                    let mut creator = User::new(Username::from("creator-a"), "token-123".into());
                    creator.github_user_id = resolve_response.user.github_user_id;
                    creator
                },
                "Initial notes".into(),
                IssueStatus::Closed,
                Some("team-a".into()),
                Some(job_settings.clone()),
                Vec::new(),
                Vec::new(),
                vec![],
            ),
            None,
        );
        let create_mock = server.mock(|when, then| {
            when.method(POST)
                .path("/v1/issues")
                .json_body_obj(&create_request);
            then.status(200)
                .json_body_obj(&UpsertIssueResponse::new(issue_id("i-456")));
        });

        create_issue(
            &client,
            IssueType::MergeRequest,
            IssueStatus::Closed,
            Vec::new(),
            vec![],
            Some("team-a".into()),
            None,
            "New issue description".into(),
            Some("Initial notes".into()),
            Some("dourolabs/example".into()),
            Some("https://example.com/service.git".into()),
            Some("worker:latest".into()),
            Some("feature/job-settings".into()),
            Some(4),
        )
        .await
        .unwrap();

        resolve_mock.assert();
        create_mock.assert();
        assert_eq!(resolve_mock.hits(), 1);
        assert_eq!(create_mock.hits(), 1);
        match original_home {
            Some(value) => env::set_var("HOME", value),
            None => env::remove_var("HOME"),
        }
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn create_issue_rejects_creator_override_mismatch() {
        let _guard = test_env::lock();
        let server = MockServer::start();
        let client = metis_client(&server);
        let original_home = env::var_os("HOME");
        let temp = tempdir().expect("tempdir");
        env::set_var("HOME", temp.path());
        let auth_token_path = temp.path().join(".local/share/metis/auth-token");
        fs::create_dir_all(auth_token_path.parent().expect("auth token parent"))
            .expect("create auth token dir");
        fs::write(&auth_token_path, "token-123").expect("write auth token");

        let resolve_request = ResolveUserRequest::new("token-123".into());
        let resolve_response =
            ResolveUserResponse::new(UserSummary::new(Username::from("creator-a")));
        let resolve_mock = server.mock(|when, then| {
            when.method(POST)
                .path("/v1/users/resolve")
                .json_body_obj(&resolve_request);
            then.status(200).json_body_obj(&resolve_response);
        });

        let result = create_issue(
            &client,
            IssueType::Bug,
            IssueStatus::Open,
            Vec::new(),
            Vec::new(),
            None,
            Some("creator-b".into()),
            "Description".into(),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;

        resolve_mock.assert();
        assert_eq!(resolve_mock.hits(), 1);
        let error = result.unwrap_err().to_string();
        assert!(error.contains("Creator override must match"));

        match original_home {
            Some(value) => env::set_var("HOME", value),
            None => env::remove_var("HOME"),
        }
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn create_issue_uses_parent_creator_for_child_dependency() {
        let server = MockServer::start();
        let client = metis_client(&server);
        let parent_id = issue_id("i-1");
        let dependencies = vec![IssueDependency::new(
            IssueDependencyType::ChildOf,
            parent_id.clone(),
        )];
        let patch_ids = vec![patch_id("p-123")];

        let parent_issue = IssueRecord::new(
            parent_id.clone(),
            Issue::new(
                IssueType::Task,
                "Parent issue".into(),
                user("parent-owner"),
                String::new(),
                IssueStatus::Open,
                None,
                None,
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
        );

        let get_parent_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{parent_id}").as_str());
            then.status(200).json_body_obj(&parent_issue);
        });

        let create_request = UpsertIssueRequest::new(
            Issue::new(
                IssueType::MergeRequest,
                "New issue description".into(),
                user("parent-owner"),
                "Initial notes".into(),
                IssueStatus::Closed,
                Some("team-a".into()),
                None,
                Vec::new(),
                dependencies.clone(),
                patch_ids.clone(),
            ),
            None,
        );
        let create_mock = server.mock(|when, then| {
            when.method(POST)
                .path("/v1/issues")
                .json_body_obj(&create_request);
            then.status(200)
                .json_body_obj(&UpsertIssueResponse::new(issue_id("i-456")));
        });

        create_issue(
            &client,
            IssueType::MergeRequest,
            IssueStatus::Closed,
            dependencies.clone(),
            patch_ids.clone(),
            Some("team-a".into()),
            None,
            "New issue description".into(),
            Some("Initial notes".into()),
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        get_parent_mock.assert();
        create_mock.assert();
        assert_eq!(get_parent_mock.hits(), 1);
        assert_eq!(create_mock.hits(), 1);
    }

    #[tokio::test]
    async fn create_issue_requires_description() {
        let server = MockServer::start();
        let client = metis_client(&server);
        assert!(create_issue(
            &client,
            IssueType::Bug,
            IssueStatus::Open,
            vec![],
            Vec::new(),
            None,
            None,
            "   ".into(),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn create_issue_rejects_empty_assignee() {
        let server = MockServer::start();
        let client = metis_client(&server);
        assert!(create_issue(
            &client,
            IssueType::Bug,
            IssueStatus::Open,
            vec![],
            Vec::new(),
            Some("   ".into()),
            None,
            "Valid description".into(),
            None,
            None,
            None,
            None,
            None,
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
        let server = MockServer::start();
        let client = metis_client(&server);
        let target_issue_id = issue_id("i-9");
        let job_settings = sample_job_settings();
        let current_issue = api_issue_record(
            "i-9",
            IssueType::Task,
            "Initial issue",
            IssueStatus::Open,
            Some("owner-a"),
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                issue_id("i-1"),
            )],
            Vec::new(),
        );
        let updated_request = UpsertIssueRequest::new(
            Issue::new(
                IssueType::Bug,
                "Updated issue description".into(),
                empty_user(),
                "New progress".into(),
                IssueStatus::Closed,
                Some("owner-b".into()),
                Some(job_settings.clone()),
                Vec::new(),
                vec![IssueDependency::new(
                    IssueDependencyType::BlockedOn,
                    issue_id("i-2"),
                )],
                vec![patch_id("p-3")],
            ),
            None,
        );
        let get_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{target_issue_id}").as_str());
            then.status(200).json_body_obj(&current_issue);
        });
        let update_mock = server.mock(|when, then| {
            when.method(PUT)
                .path(format!("/v1/issues/{target_issue_id}").as_str())
                .json_body_obj(&updated_request);
            then.status(200)
                .json_body_obj(&UpsertIssueResponse::new(target_issue_id.clone()));
        });

        update_issue(
            &client,
            target_issue_id,
            Some(IssueType::Bug),
            Some(IssueStatus::Closed),
            Some("owner-b".into()),
            false,
            None,
            Some("Updated issue description".into()),
            vec![IssueDependency::new(
                IssueDependencyType::BlockedOn,
                issue_id("i-2"),
            )],
            false,
            vec![patch_id("p-3")],
            false,
            Some("New progress".into()),
            false,
            Some("dourolabs/example".into()),
            Some("https://example.com/service.git".into()),
            Some("worker:123".into()),
            Some("main".into()),
            Some(5),
            false,
        )
        .await
        .unwrap();

        get_mock.assert();
        update_mock.assert();
        assert_eq!(get_mock.hits(), 1);
        assert_eq!(update_mock.hits(), 1);
    }

    #[tokio::test]
    async fn update_issue_allows_clearing_assignee_and_dependencies() {
        let server = MockServer::start();
        let client = metis_client(&server);
        let target_issue_id = issue_id("i-10");
        let current_issue = IssueRecord::new(
            target_issue_id.clone(),
            Issue::new(
                IssueType::Feature,
                "Existing issue".into(),
                empty_user(),
                "Started work".into(),
                IssueStatus::InProgress,
                Some("owner-a".into()),
                None,
                Vec::new(),
                vec![IssueDependency::new(
                    IssueDependencyType::BlockedOn,
                    issue_id("i-5"),
                )],
                Vec::new(),
            ),
        );
        let update_request = UpsertIssueRequest::new(
            Issue::new(
                IssueType::Feature,
                "Existing issue".into(),
                empty_user(),
                String::new(),
                IssueStatus::InProgress,
                None,
                None,
                Vec::new(),
                vec![],
                Vec::new(),
            ),
            None,
        );
        let get_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{target_issue_id}").as_str());
            then.status(200).json_body_obj(&current_issue);
        });
        let update_mock = server.mock(|when, then| {
            when.method(PUT)
                .path(format!("/v1/issues/{target_issue_id}").as_str())
                .json_body_obj(&update_request);
            then.status(200)
                .json_body_obj(&UpsertIssueResponse::new(target_issue_id.clone()));
        });

        update_issue(
            &client,
            target_issue_id,
            None,
            None,
            None,
            true,
            None,
            None,
            vec![],
            true,
            vec![],
            true,
            None,
            true,
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .await
        .unwrap();

        get_mock.assert();
        update_mock.assert();
        assert_eq!(get_mock.hits(), 1);
        assert_eq!(update_mock.hits(), 1);
    }

    #[tokio::test]
    async fn update_issue_allows_clearing_job_settings() {
        let server = MockServer::start();
        let client = metis_client(&server);
        let target_issue_id = issue_id("i-11");
        let job_settings = sample_job_settings();
        let current_issue = IssueRecord::new(
            target_issue_id.clone(),
            Issue::new(
                IssueType::Feature,
                "Existing issue".into(),
                empty_user(),
                "Started work".into(),
                IssueStatus::InProgress,
                Some("owner-a".into()),
                Some(job_settings),
                Vec::new(),
                vec![IssueDependency::new(
                    IssueDependencyType::BlockedOn,
                    issue_id("i-5"),
                )],
                Vec::new(),
            ),
        );
        let update_request = UpsertIssueRequest::new(
            Issue::new(
                IssueType::Feature,
                "Existing issue".into(),
                empty_user(),
                "Started work".into(),
                IssueStatus::InProgress,
                Some("owner-a".into()),
                None,
                Vec::new(),
                vec![IssueDependency::new(
                    IssueDependencyType::BlockedOn,
                    issue_id("i-5"),
                )],
                Vec::new(),
            ),
            None,
        );
        let get_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{target_issue_id}").as_str());
            then.status(200).json_body_obj(&current_issue);
        });
        let update_mock = server.mock(|when, then| {
            when.method(PUT)
                .path(format!("/v1/issues/{target_issue_id}").as_str())
                .json_body_obj(&update_request);
            then.status(200)
                .json_body_obj(&UpsertIssueResponse::new(target_issue_id.clone()));
        });

        update_issue(
            &client,
            target_issue_id,
            None,
            None,
            None,
            false,
            None,
            None,
            vec![],
            false,
            vec![],
            false,
            None,
            false,
            None,
            None,
            None,
            None,
            None,
            true,
        )
        .await
        .unwrap();

        get_mock.assert();
        update_mock.assert();
        assert_eq!(get_mock.hits(), 1);
        assert_eq!(update_mock.hits(), 1);
    }

    #[test]
    fn pretty_prints_human_readable_issues() {
        let issues = vec![
            IssueRecord::new(
                issue_id("i-1"),
                Issue::new(
                    IssueType::Bug,
                    "First issue\nwith context".into(),
                    empty_user(),
                    "Working on repro".into(),
                    IssueStatus::Open,
                    Some("owner-a".into()),
                    None,
                    Vec::new(),
                    vec![IssueDependency::new(
                        IssueDependencyType::BlockedOn,
                        issue_id("i-99"),
                    )],
                    Vec::new(),
                ),
            ),
            IssueRecord::new(
                issue_id("i-2"),
                Issue::new(
                    IssueType::Feature,
                    "Follow-up work".into(),
                    empty_user(),
                    String::new(),
                    IssueStatus::InProgress,
                    None,
                    None,
                    Vec::new(),
                    vec![],
                    Vec::new(),
                ),
            ),
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

    #[tokio::test]
    async fn todo_command_fetches_existing_list() {
        let server = MockServer::start();
        let client = metis_client(&server);
        let issue_id = issue_id("i-todo");
        let todo_list = vec![
            TodoItem::new("write docs".into(), false),
            TodoItem::new("add tests".into(), true),
        ];
        let get_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{issue_id}").as_str());
            then.status(200).json_body_obj(&IssueRecord::new(
                issue_id.clone(),
                Issue::new(
                    IssueType::Task,
                    "has todos".into(),
                    empty_user(),
                    String::new(),
                    IssueStatus::Open,
                    None,
                    None,
                    todo_list.clone(),
                    vec![],
                    Vec::new(),
                ),
            ));
        });

        let resolved = resolve_todo_list(&client, &issue_id, None, None, None, None)
            .await
            .unwrap();

        get_mock.assert();
        assert_eq!(get_mock.hits(), 1);
        assert_eq!(resolved, todo_list);
    }

    #[tokio::test]
    async fn todo_command_adds_item_and_parses_done_prefix() {
        let server = MockServer::start();
        let client = metis_client(&server);
        let issue_id = issue_id("i-add");
        let add_request = AddTodoItemRequest::new("finish docs".into(), true);
        let add_mock = server.mock(|when, then| {
            when.method(POST)
                .path(format!("/v1/issues/{issue_id}/todo-items").as_str())
                .json_body_obj(&add_request);
            then.status(200)
                .json_body_obj(&TodoListResponse::new(issue_id.clone(), Vec::new()));
        });

        let updated = resolve_todo_list(
            &client,
            &issue_id,
            Some("[x] finish docs".into()),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        add_mock.assert();
        assert_eq!(add_mock.hits(), 1);
        assert!(updated.is_empty());
    }

    #[tokio::test]
    async fn todo_command_marks_item_done_and_undone() {
        let server = MockServer::start();
        let client = metis_client(&server);
        let issue_id = issue_id("i-done");
        let mark_done_request = SetTodoItemStatusRequest::new(true);
        let mark_done_mock = server.mock(|when, then| {
            when.method(POST)
                .path(format!("/v1/issues/{issue_id}/todo-items/1").as_str())
                .json_body_obj(&mark_done_request);
            then.status(200).json_body_obj(&TodoListResponse::new(
                issue_id.clone(),
                vec![TodoItem::new("first".into(), true)],
            ));
        });

        let done_list = resolve_todo_list(&client, &issue_id, None, Some(1), None, None)
            .await
            .unwrap();
        mark_done_mock.assert();
        assert_eq!(mark_done_mock.hits(), 1);
        assert!(done_list[0].is_done);

        let mark_undone_request = SetTodoItemStatusRequest::new(false);
        let mark_undone_mock = server.mock(|when, then| {
            when.method(POST)
                .path(format!("/v1/issues/{issue_id}/todo-items/1").as_str())
                .json_body_obj(&mark_undone_request);
            then.status(200).json_body_obj(&TodoListResponse::new(
                issue_id.clone(),
                vec![TodoItem::new("first".into(), false)],
            ));
        });

        let undone_list = resolve_todo_list(&client, &issue_id, None, None, Some(1), None)
            .await
            .unwrap();
        mark_undone_mock.assert();
        assert_eq!(mark_undone_mock.hits(), 1);
        assert!(!undone_list[0].is_done);
    }

    #[tokio::test]
    async fn todo_command_replaces_list_with_parsed_items() {
        let server = MockServer::start();
        let client = metis_client(&server);
        let issue_id = issue_id("i-replace");
        let parsed = vec![
            TodoItem::new("first item".into(), false),
            TodoItem::new("second".into(), true),
        ];
        let replace_mock = server.mock(|when, then| {
            when.method(PUT)
                .path(format!("/v1/issues/{issue_id}/todo-items").as_str())
                .json_body_obj(&ReplaceTodoListRequest::new(parsed.clone()));
            then.status(200)
                .json_body_obj(&TodoListResponse::new(issue_id.clone(), parsed.clone()));
        });

        let resolved = resolve_todo_list(
            &client,
            &issue_id,
            None,
            None,
            None,
            Some(vec!["first item".into(), "[x] second".into()]),
        )
        .await
        .unwrap();

        replace_mock.assert();
        assert_eq!(replace_mock.hits(), 1);
        assert_eq!(resolved, parsed);
    }

    #[test]
    fn render_todo_list_formats_output() {
        let todo_list = vec![
            TodoItem::new("write docs".into(), false),
            TodoItem::new("add tests\nwith details".into(), true),
        ];
        let mut output = Vec::new();
        render_todo_list(&issue_id("i-render"), &todo_list, &mut output).unwrap();
        let rendered = String::from_utf8(output).unwrap();

        assert!(rendered.contains("Todos for issue i-render:"));
        assert!(rendered.contains("1. [ ] write docs"));
        assert!(rendered.contains("2. [x] add tests"));
        assert!(
            rendered.contains("         with details"),
            "continuation lines should be indented"
        );
    }

    #[test]
    fn describe_issue_pretty_printer_includes_sections() {
        let main_patch_id = patch_id("p-main");
        let main_patch_record = PatchRecord::new(
            main_patch_id.clone(),
            Patch::new(
                "main patch".into(),
                "desc".into(),
                sample_diff(),
                Default::default(),
                false,
                None,
                Vec::new(),
                sample_repo_name(),
                None,
            ),
        );
        let description = IssueDescription {
            issue: IssueWithPatches {
                issue: IssueRecord::new(
                    issue_id("i-main"),
                    Issue::new(
                        IssueType::Task,
                        "Main issue".into(),
                        empty_user(),
                        String::new(),
                        IssueStatus::Open,
                        Some("owner".into()),
                        None,
                        Vec::new(),
                        vec![],
                        vec![main_patch_id],
                    ),
                ),
                patches: vec![main_patch_record],
            },
            parents: vec![IssueWithPatches {
                issue: IssueRecord::new(
                    issue_id("i-parent"),
                    Issue::new(
                        IssueType::Feature,
                        "Parent".into(),
                        empty_user(),
                        String::new(),
                        IssueStatus::Open,
                        None,
                        None,
                        Vec::new(),
                        vec![],
                        Vec::new(),
                    ),
                ),
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
    fn describe_issue_pretty_printer_includes_progress() {
        let description = IssueDescription {
            issue: IssueWithPatches {
                issue: IssueRecord::new(
                    issue_id("i-main"),
                    Issue::new(
                        IssueType::Task,
                        "Main issue".into(),
                        empty_user(),
                        "Main progress".into(),
                        IssueStatus::Open,
                        Some("owner".into()),
                        None,
                        Vec::new(),
                        vec![],
                        Vec::new(),
                    ),
                ),
                patches: Vec::new(),
            },
            parents: vec![IssueWithPatches {
                issue: IssueRecord::new(
                    issue_id("i-parent"),
                    Issue::new(
                        IssueType::Feature,
                        "Parent".into(),
                        empty_user(),
                        String::new(),
                        IssueStatus::Open,
                        None,
                        None,
                        Vec::new(),
                        vec![],
                        Vec::new(),
                    ),
                ),
                patches: Vec::new(),
            }],
            children: vec![IssueWithPatches {
                issue: IssueRecord::new(
                    issue_id("i-child"),
                    Issue::new(
                        IssueType::Bug,
                        "Child".into(),
                        empty_user(),
                        "Child update".into(),
                        IssueStatus::InProgress,
                        None,
                        None,
                        Vec::new(),
                        vec![],
                        Vec::new(),
                    ),
                ),
                patches: Vec::new(),
            }],
        };

        let mut output = Vec::new();
        print_issue_description_pretty(&description, &mut output).unwrap();
        let rendered = String::from_utf8(output).unwrap();

        assert!(rendered.contains("Progress:\n    Main progress"));
        assert!(rendered.contains("Parents:\n  Issue i-parent (feature, open)\n  Creator: \n  Assignee: -\n  Description:\n    Parent\n  Progress:\n    -"));
        assert!(rendered.contains("Children (transitive):\n  Issue i-child (bug, in-progress)\n  Creator: \n  Assignee: -\n  Description:\n    Child\n  Progress:\n    Child update"));
    }

    #[test]
    fn describe_issue_pretty_printer_shows_todos_for_root_issue_only() {
        let root_todos = vec![
            TodoItem::new("root todo".into(), false),
            TodoItem::new("root done".into(), true),
        ];
        let description = IssueDescription {
            issue: IssueWithPatches {
                issue: IssueRecord::new(
                    issue_id("i-main"),
                    Issue::new(
                        IssueType::Task,
                        "Main issue".into(),
                        empty_user(),
                        String::new(),
                        IssueStatus::Open,
                        Some("owner".into()),
                        None,
                        root_todos.clone(),
                        vec![],
                        Vec::new(),
                    ),
                ),
                patches: Vec::new(),
            },
            parents: vec![IssueWithPatches {
                issue: IssueRecord::new(
                    issue_id("i-parent"),
                    Issue::new(
                        IssueType::Task,
                        "Parent description".into(),
                        empty_user(),
                        String::new(),
                        IssueStatus::Open,
                        None,
                        None,
                        vec![TodoItem::new("parent todo".into(), false)],
                        vec![],
                        Vec::new(),
                    ),
                ),
                patches: Vec::new(),
            }],
            children: vec![IssueWithPatches {
                issue: IssueRecord::new(
                    issue_id("i-child"),
                    Issue::new(
                        IssueType::Bug,
                        "Child description".into(),
                        empty_user(),
                        String::new(),
                        IssueStatus::Open,
                        None,
                        None,
                        vec![TodoItem::new("child todo".into(), true)],
                        vec![],
                        Vec::new(),
                    ),
                ),
                patches: Vec::new(),
            }],
        };

        let mut output = Vec::new();
        print_issue_description_pretty(&description, &mut output).unwrap();
        let rendered = String::from_utf8(output).unwrap();

        assert!(rendered.contains("Todos:\n    1. [ ] root todo\n    2. [x] root done"));
        assert!(!rendered.contains("parent todo"));
        assert!(!rendered.contains("child todo"));
    }

    #[test]
    fn describe_issue_pretty_printer_shows_review_summary() {
        let main_patch_id = patch_id("p-main");
        let earliest_review = Utc.with_ymd_and_hms(2024, 5, 1, 11, 50, 0).unwrap();
        let latest_review = earliest_review + Duration::minutes(10);
        let patch_reviews = vec![
            Review::new(
                "needs work".to_string(),
                false,
                "alex".to_string(),
                Some(earliest_review),
            ),
            Review::new(
                "fixed now".to_string(),
                false,
                "sam".to_string(),
                Some(earliest_review + Duration::minutes(5)),
            ),
            Review::new(
                "ship it".to_string(),
                true,
                "sam".to_string(),
                Some(latest_review),
            ),
        ];
        let description = IssueDescription {
            issue: IssueWithPatches {
                issue: IssueRecord::new(
                    issue_id("i-main"),
                    Issue::new(
                        IssueType::Task,
                        "Main issue".into(),
                        empty_user(),
                        String::new(),
                        IssueStatus::Open,
                        Some("owner".into()),
                        None,
                        Vec::new(),
                        vec![],
                        vec![main_patch_id.clone()],
                    ),
                ),
                patches: vec![PatchRecord::new(
                    main_patch_id,
                    Patch::new(
                        "main patch".into(),
                        "desc".into(),
                        sample_diff(),
                        Default::default(),
                        false,
                        None,
                        patch_reviews,
                        sample_repo_name(),
                        None,
                    ),
                )],
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
