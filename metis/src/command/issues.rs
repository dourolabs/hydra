use crate::client::MetisClientInterface;
use anyhow::{bail, Context, Result};
use clap::Subcommand;
use metis_common::{
    artifacts::{
        Issue, IssueDependency, IssueDependencyType, IssueRecord, IssueStatus, IssueType,
        SearchIssuesQuery, UpsertIssueRequest,
    },
    MetisId,
};
use std::io::{self, Write};
use std::str::FromStr;

#[derive(Debug, Subcommand)]
pub enum IssueCommands {
    /// List Metis issues (artifacts of type Issue).
    List {
        /// Filter by issue ID.
        #[arg(long, value_name = "ISSUE_ID", conflicts_with = "query")]
        id: Option<MetisId>,

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
    },
    /// Create a new issue artifact.
    Create {
        /// Issue type: bug, feature, task, chore, or merge-request (defaults to task).
        #[arg(long, value_name = "ISSUE_TYPE", default_value_t = IssueType::Task)]
        r#type: IssueType,

        /// Issue status: open, in-progress, or closed (defaults to open).
        #[arg(long, value_name = "ISSUE_STATUS", default_value_t = IssueStatus::Open)]
        status: IssueStatus,

        /// Issue dependencies in the format dependency-type:ISSUE_ID where dependency-type is child-of or blocked-on (e.g. child-of:ISSUE-123).
        #[arg(long = "deps", value_name = "TYPE:ISSUE_ID", value_parser = parse_issue_dependency)]
        dependencies: Vec<IssueDependency>,

        /// Assignee for the issue.
        #[arg(long, value_name = "ASSIGNEE")]
        assignee: Option<String>,

        /// Description for the issue.
        #[arg(value_name = "DESCRIPTION")]
        description: String,
    },
    /// Update an existing issue artifact.
    Update {
        /// Issue ID to update.
        #[arg(value_name = "ISSUE_ID")]
        id: MetisId,

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

        /// Replace dependencies with the provided set in the format TYPE:ISSUE_ID (e.g. child-of:ISSUE-123).
        #[arg(long = "deps", value_name = "TYPE:ISSUE_ID", value_parser = parse_issue_dependency, conflicts_with = "clear_dependencies")]
        dependencies: Vec<IssueDependency>,

        /// Remove all dependencies from the issue.
        #[arg(long)]
        clear_dependencies: bool,
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
        } => {
            let issues = fetch_issues(client, id, r#type, status, assignee, query).await?;
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
            assignee,
            description,
        } => create_issue(client, r#type, status, dependencies, assignee, description).await,
        IssueCommands::Update {
            id,
            r#type,
            status,
            assignee,
            clear_assignee,
            description,
            dependencies,
            clear_dependencies,
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
            )
            .await
        }
    }
}

async fn fetch_issues(
    client: &dyn MetisClientInterface,
    id: Option<MetisId>,
    issue_type: Option<IssueType>,
    status: Option<IssueStatus>,
    assignee: Option<String>,
    query: Option<String>,
) -> Result<Vec<IssueRecord>> {
    if let Some(id) = id {
        let issue_id = id.trim();
        if issue_id.is_empty() {
            bail!("Issue ID must not be empty.");
        }
        let issue_id: MetisId = issue_id.to_string();

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
    assignee: Option<String>,
    description: String,
) -> Result<()> {
    let description = description.trim();
    if description.is_empty() {
        bail!("Issue description must not be empty.");
    }

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
            status,
            assignee,
            dependencies,
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
    id: MetisId,
    issue_type: Option<IssueType>,
    status: Option<IssueStatus>,
    assignee: Option<String>,
    clear_assignee: bool,
    description: Option<String>,
    dependencies: Vec<IssueDependency>,
    clear_dependencies: bool,
) -> Result<()> {
    let issue_id = id.trim();
    if issue_id.is_empty() {
        bail!("Issue ID must not be empty.");
    }
    let issue_id = issue_id.to_string();

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

    let no_changes = issue_type.is_none()
        && status.is_none()
        && assignee.is_none()
        && description.is_none()
        && dependencies_update.is_none();
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
        status: status.unwrap_or(current.issue.status),
        assignee: assignee.unwrap_or(current.issue.assignee),
        dependencies: dependencies_update.unwrap_or(current.issue.dependencies),
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

fn parse_issue_dependency(raw: &str) -> Result<IssueDependency, String> {
    let (dependency_type, issue_id) = raw
        .split_once(':')
        .ok_or_else(|| "dependency must be in the format TYPE:ISSUE_ID".to_string())?;

    let dependency_type =
        IssueDependencyType::from_str(dependency_type).map_err(|err| err.to_string())?;
    let issue_id = issue_id.trim();
    if issue_id.is_empty() {
        return Err("dependency issue id must not be empty".to_string());
    }

    Ok(IssueDependency {
        dependency_type,
        issue_id: issue_id.to_string(),
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
            status,
            assignee,
            dependencies,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MockMetisClient;
    use metis_common::artifacts::{
        Issue, IssueRecord, ListIssuesResponse, SearchIssuesQuery, UpsertIssueRequest,
        UpsertIssueResponse,
    };

    #[tokio::test]
    async fn list_issues_filters_by_query_and_prints_jsonl() {
        let client = MockMetisClient::default();
        client.push_list_issues_response(ListIssuesResponse {
            issues: vec![IssueRecord {
                id: "issue-1".into(),
                issue: Issue {
                    issue_type: IssueType::Bug,
                    description: "First issue".into(),
                    status: IssueStatus::Open,
                    assignee: None,
                    dependencies: vec![],
                },
            }],
        });

        let artifacts = fetch_issues(
            &client,
            None,
            Some(IssueType::Bug),
            Some(IssueStatus::Open),
            None,
            Some("bug".into()),
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
            }]
        );

        let mut output = Vec::new();
        print_issues_jsonl(&artifacts, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("\"id\":\"issue-1\""));
        assert!(!output.contains("\"id\":\"issue-2\""));
    }

    #[tokio::test]
    async fn list_issues_by_id_returns_single_issue() {
        let client = MockMetisClient::default();
        client.push_get_issue_response(IssueRecord {
            id: "issue-123".into(),
            issue: Issue {
                issue_type: IssueType::Task,
                description: "Edge case bug".into(),
                status: IssueStatus::InProgress,
                assignee: None,
                dependencies: vec![],
            },
        });

        let artifacts = fetch_issues(
            &client,
            Some("issue-123".into()),
            Some(IssueType::Task),
            Some(IssueStatus::InProgress),
            None,
            None,
        )
        .await
        .unwrap();

        assert_eq!(client.recorded_get_issue_requests(), vec!["issue-123"]);
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].id, "issue-123");
    }

    #[tokio::test]
    async fn list_issues_filters_by_assignee() {
        let client = MockMetisClient::default();
        client.push_list_issues_response(ListIssuesResponse {
            issues: vec![IssueRecord {
                id: "issue-7".into(),
                issue: Issue {
                    issue_type: IssueType::Task,
                    description: "Edge case bug".into(),
                    status: IssueStatus::Open,
                    assignee: Some("owner-a".into()),
                    dependencies: vec![],
                },
            }],
        });

        let artifacts = fetch_issues(&client, None, None, None, Some("OWNER-A".into()), None)
            .await
            .unwrap();

        assert_eq!(
            client.recorded_list_issue_queries(),
            vec![SearchIssuesQuery {
                issue_type: None,
                status: None,
                assignee: Some("OWNER-A".into()),
                q: None,
            }]
        );
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].id, "issue-7");
    }

    #[tokio::test]
    async fn create_issue_submits_issue_artifact() {
        let client = MockMetisClient::default();
        client.push_upsert_issue_response(UpsertIssueResponse {
            issue_id: "issue-456".into(),
        });

        let dependencies = vec![IssueDependency {
            dependency_type: IssueDependencyType::ChildOf,
            issue_id: "issue-1".into(),
        }];

        create_issue(
            &client,
            IssueType::MergeRequest,
            IssueStatus::Closed,
            dependencies.clone(),
            Some("team-a".into()),
            "New issue description".into(),
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
                        assignee: Some("team-a".into()),
                        dependencies,
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
            None,
            "   ".into()
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
            Some("   ".into()),
            "Valid description".into()
        )
        .await
        .is_err());
    }

    #[test]
    fn parse_issue_dependency_parses_type_and_id() {
        let dependency = parse_issue_dependency("child-of:ISSUE-42").unwrap();
        assert_eq!(dependency.dependency_type, IssueDependencyType::ChildOf);
        assert_eq!(dependency.issue_id, "ISSUE-42");
    }

    #[tokio::test]
    async fn update_issue_modifies_requested_fields() {
        let client = MockMetisClient::default();
        client.push_get_issue_response(IssueRecord {
            id: "issue-9".into(),
            issue: Issue {
                issue_type: IssueType::Task,
                description: "Initial issue".into(),
                status: IssueStatus::Open,
                assignee: Some("owner-a".into()),
                dependencies: vec![IssueDependency {
                    dependency_type: IssueDependencyType::ChildOf,
                    issue_id: "issue-1".into(),
                }],
            },
        });
        client.push_upsert_issue_response(UpsertIssueResponse {
            issue_id: "issue-9".into(),
        });

        update_issue(
            &client,
            "issue-9".into(),
            Some(IssueType::Bug),
            Some(IssueStatus::Closed),
            Some("owner-b".into()),
            false,
            Some("Updated issue description".into()),
            vec![IssueDependency {
                dependency_type: IssueDependencyType::BlockedOn,
                issue_id: "issue-2".into(),
            }],
            false,
        )
        .await
        .unwrap();

        assert_eq!(client.recorded_get_issue_requests(), vec!["issue-9"]);
        assert_eq!(
            client.recorded_issue_upserts(),
            vec![(
                Some("issue-9".into()),
                UpsertIssueRequest {
                    issue: Issue {
                        issue_type: IssueType::Bug,
                        description: "Updated issue description".into(),
                        status: IssueStatus::Closed,
                        assignee: Some("owner-b".into()),
                        dependencies: vec![IssueDependency {
                            dependency_type: IssueDependencyType::BlockedOn,
                            issue_id: "issue-2".into(),
                        }],
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
            id: "issue-10".into(),
            issue: Issue {
                issue_type: IssueType::Feature,
                description: "Existing issue".into(),
                status: IssueStatus::InProgress,
                assignee: Some("owner-a".into()),
                dependencies: vec![IssueDependency {
                    dependency_type: IssueDependencyType::BlockedOn,
                    issue_id: "issue-5".into(),
                }],
            },
        });
        client.push_upsert_issue_response(UpsertIssueResponse {
            issue_id: "issue-10".into(),
        });

        update_issue(
            &client,
            "issue-10".into(),
            None,
            None,
            None,
            true,
            None,
            vec![],
            true,
        )
        .await
        .unwrap();

        assert_eq!(
            client.recorded_issue_upserts(),
            vec![(
                Some("issue-10".into()),
                UpsertIssueRequest {
                    issue: Issue {
                        issue_type: IssueType::Feature,
                        description: "Existing issue".into(),
                        status: IssueStatus::InProgress,
                        assignee: None,
                        dependencies: vec![],
                    },
                    job_id: None,
                }
            )]
        );
    }

    #[test]
    fn pretty_prints_human_readable_issues() {
        let artifacts = vec![
            IssueRecord {
                id: "issue-1".into(),
                issue: Issue {
                    issue_type: IssueType::Bug,
                    description: "First issue\nwith context".into(),
                    status: IssueStatus::Open,
                    assignee: Some("owner-a".into()),
                    dependencies: vec![IssueDependency {
                        dependency_type: IssueDependencyType::BlockedOn,
                        issue_id: "issue-99".into(),
                    }],
                },
            },
            IssueRecord {
                id: "issue-2".into(),
                issue: Issue {
                    issue_type: IssueType::Feature,
                    description: "Follow-up work".into(),
                    status: IssueStatus::InProgress,
                    assignee: None,
                    dependencies: vec![],
                },
            },
        ];

        let mut output = Vec::new();
        print_issues_pretty(&artifacts, &mut output).unwrap();
        let rendered = String::from_utf8(output).unwrap();

        assert!(rendered.contains("Issue issue-1 (bug, open)"));
        assert!(rendered.contains("Assignee: owner-a"));
        assert!(rendered.contains("Description:\n  First issue\n  with context"));
        assert!(rendered.contains("Dependencies:\n  - blocked-on issue-99"));
        assert!(rendered.contains("Issue issue-2 (feature, in-progress)"));
        assert!(rendered.contains("Assignee: -"));
        assert!(rendered.contains("Dependencies: none"));
        assert!(rendered.contains("Follow-up work"));
    }
}
