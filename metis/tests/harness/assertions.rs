#![allow(dead_code)]

use std::future::Future;
use std::time::Duration;

use anyhow::{bail, Result};
use metis_common::{
    issues::{CommonIssueFields, CommonIssueRecord, IssueDependencyType, IssueStatus, IssueType},
    jobs::JobVersionRecord,
    patches::{PatchStatus, PatchVersionRecord},
    task_status::Status,
    IssueId,
};

// ── IssueAssertions ─────────────────────────────────────────────────

/// Structured assertions on issue records.
///
/// Blanket-implemented for any type that implements [`CommonIssueRecord`],
/// so it works with both `IssueVersionRecord` and `IssueSummaryRecord`:
/// ```ignore
/// let issue = user.get_issue(&id).await?;
/// issue.assert_status(IssueStatus::Closed);
/// issue.assert_todo_count(3);
/// ```
pub trait IssueAssertions: CommonIssueRecord {
    /// Assert the issue has the expected status.
    fn assert_status(&self, expected: IssueStatus);

    /// Assert that at least one issue in `all_issues` is a child of this issue,
    /// has a description containing `desc_contains`, and has the given `status`.
    ///
    /// A child is an issue whose dependencies include a `ChildOf` edge pointing
    /// to this issue's ID. The `all_issues` slice can be any record type
    /// implementing [`CommonIssueRecord`].
    fn assert_has_child_with_status<R: CommonIssueRecord>(
        &self,
        all_issues: &[R],
        desc_contains: &str,
        status: IssueStatus,
    );

    /// Assert the issue's todo list has exactly `expected` items.
    fn assert_todo_count(&self, expected: usize);

    /// Assert the issue has at least one patch attached.
    fn assert_has_patch(&self);
}

impl<T: CommonIssueRecord> IssueAssertions for T {
    fn assert_status(&self, expected: IssueStatus) {
        assert_eq!(
            self.issue().status(),
            expected,
            "issue {}: expected status {:?}, got {:?}",
            self.issue_id(),
            expected,
            self.issue().status()
        );
    }

    fn assert_has_child_with_status<R: CommonIssueRecord>(
        &self,
        all_issues: &[R],
        desc_contains: &str,
        status: IssueStatus,
    ) {
        let children: Vec<&R> = all_issues
            .iter()
            .filter(|issue| {
                issue.issue().dependencies().iter().any(|dep| {
                    dep.dependency_type == IssueDependencyType::ChildOf
                        && dep.issue_id == *self.issue_id()
                })
            })
            .collect();

        let matching = children.iter().find(|child| {
            child.issue().description().contains(desc_contains) && child.issue().status() == status
        });

        if matching.is_none() {
            let child_summaries: Vec<String> = children
                .iter()
                .map(|c| {
                    format!(
                        "  {} (status={:?}, desc={:?})",
                        c.issue_id(),
                        c.issue().status(),
                        c.issue().description()
                    )
                })
                .collect();
            panic!(
                "issue {}: expected a child with description containing {:?} and status {:?}, \
                 but no matching child found.\nchildren:\n{}",
                self.issue_id(),
                desc_contains,
                status,
                if child_summaries.is_empty() {
                    "  (none)".to_string()
                } else {
                    child_summaries.join("\n")
                }
            );
        }
    }

    fn assert_todo_count(&self, expected: usize) {
        let actual = self.issue().todo_list().len();
        assert_eq!(
            actual,
            expected,
            "issue {}: expected {} todo items, got {}",
            self.issue_id(),
            expected,
            actual
        );
    }

    fn assert_has_patch(&self) {
        assert!(
            !self.issue().patches().is_empty(),
            "issue {}: expected at least one patch, but patches list is empty",
            self.issue_id()
        );
    }
}

// ── PatchAssertions ─────────────────────────────────────────────────

/// Structured assertions on patch records.
///
/// ```ignore
/// let patch = user.get_patch(&id).await?;
/// patch.assert_status(PatchStatus::Open);
/// patch.assert_diff_contains("fn main");
/// ```
pub trait PatchAssertions {
    /// Assert the patch has the expected status.
    fn assert_status(&self, expected: PatchStatus);

    /// Assert there is a review from `author` with the given approval state.
    fn assert_review_from(&self, author: &str, is_approved: bool);

    /// Assert the patch diff contains the given text.
    fn assert_diff_contains(&self, text: &str);
}

impl PatchAssertions for PatchVersionRecord {
    fn assert_status(&self, expected: PatchStatus) {
        assert_eq!(
            self.patch.status, expected,
            "patch {}: expected status {:?}, got {:?}",
            self.patch_id, expected, self.patch.status
        );
    }

    fn assert_review_from(&self, author: &str, is_approved: bool) {
        let review = self.patch.reviews.iter().find(|r| r.author == author);
        match review {
            Some(r) => {
                assert_eq!(
                    r.is_approved, is_approved,
                    "patch {}: review from '{}' expected is_approved={}, got is_approved={}",
                    self.patch_id, author, is_approved, r.is_approved
                );
            }
            None => {
                let authors: Vec<&str> = self
                    .patch
                    .reviews
                    .iter()
                    .map(|r| r.author.as_str())
                    .collect();
                panic!(
                    "patch {}: expected review from '{}', but only found reviews from: {:?}",
                    self.patch_id, author, authors
                );
            }
        }
    }

    fn assert_diff_contains(&self, text: &str) {
        assert!(
            self.patch.diff.contains(text),
            "patch {}: expected diff to contain {:?}, but it does not.\ndiff preview: {:?}",
            self.patch_id,
            text,
            &self.patch.diff[..self.patch.diff.len().min(200)]
        );
    }
}

// ── JobAssertions ───────────────────────────────────────────────────

/// Structured assertions on job records.
///
/// ```ignore
/// let job = user.client().get_job(&job_id).await?;
/// job.assert_status(Status::Complete);
/// job.assert_env_var("METIS_TOKEN", "secret");
/// ```
pub trait JobAssertions {
    /// Assert the job has the expected status.
    fn assert_status(&self, expected: Status);

    /// Assert the job has an environment variable with the given key and value.
    fn assert_env_var(&self, key: &str, value: &str);
}

impl JobAssertions for JobVersionRecord {
    fn assert_status(&self, expected: Status) {
        assert_eq!(
            self.task.status, expected,
            "job {}: expected status {:?}, got {:?}",
            self.job_id, expected, self.task.status
        );
    }

    fn assert_env_var(&self, key: &str, value: &str) {
        match self.task.env_vars.get(key) {
            Some(actual) => {
                assert_eq!(
                    actual, value,
                    "job {}: env var '{}' expected value {:?}, got {:?}",
                    self.job_id, key, value, actual
                );
            }
            None => {
                let keys: Vec<&str> = self.task.env_vars.keys().map(|k| k.as_str()).collect();
                panic!(
                    "job {}: expected env var '{}', but only found: {:?}",
                    self.job_id, key, keys
                );
            }
        }
    }
}

// ── Generic issue query helpers ─────────────────────────────────────

/// Find the first issue record whose description contains `desc_contains`.
///
/// Works with both `IssueVersionRecord` and `IssueSummaryRecord`.
pub fn find_issue_by_description<'a, R: CommonIssueRecord>(
    issues: &'a [R],
    desc_contains: &str,
) -> Option<&'a R> {
    issues
        .iter()
        .find(|i| i.issue().description().contains(desc_contains))
}

/// Find all issue records that are children of `parent_id` (via a `ChildOf` dependency).
///
/// Works with both `IssueVersionRecord` and `IssueSummaryRecord`.
pub fn find_children_of<'a, R: CommonIssueRecord>(
    issues: &'a [R],
    parent_id: &IssueId,
) -> Vec<&'a R> {
    issues
        .iter()
        .filter(|i| {
            i.issue().dependencies().iter().any(|d| {
                d.dependency_type == IssueDependencyType::ChildOf && d.issue_id == *parent_id
            })
        })
        .collect()
}

/// Find all child issue records of `parent_id` that match the given `issue_type`.
///
/// Works with both `IssueVersionRecord` and `IssueSummaryRecord`.
pub fn find_children_by_type<'a, R: CommonIssueRecord>(
    issues: &'a [R],
    parent_id: &IssueId,
    issue_type: IssueType,
) -> Vec<&'a R> {
    issues
        .iter()
        .filter(|i| {
            i.issue().issue_type() == issue_type
                && i.issue().dependencies().iter().any(|d| {
                    d.dependency_type == IssueDependencyType::ChildOf && d.issue_id == *parent_id
                })
        })
        .collect()
}

/// Find all child issue records of `parent_id` matching `issue_type` and `status`.
///
/// Works with both `IssueVersionRecord` and `IssueSummaryRecord`.
pub fn find_children_by_type_and_status<'a, R: CommonIssueRecord>(
    issues: &'a [R],
    parent_id: &IssueId,
    issue_type: IssueType,
    status: IssueStatus,
) -> Vec<&'a R> {
    issues
        .iter()
        .filter(|i| {
            i.issue().issue_type() == issue_type
                && i.issue().status() == status
                && i.issue().dependencies().iter().any(|d| {
                    d.dependency_type == IssueDependencyType::ChildOf && d.issue_id == *parent_id
                })
        })
        .collect()
}

// ── wait_until ──────────────────────────────────────────────────────

/// Generic async polling helper that replaces ad-hoc polling loops.
///
/// Calls `condition` repeatedly at `poll_interval` intervals until it returns
/// `true` or `timeout` is exceeded. On timeout, returns an error that includes
/// `description` for easy debugging.
///
/// # Example
///
/// ```ignore
/// wait_until(
///     Duration::from_secs(5),
///     Duration::from_millis(50),
///     "job to reach Running status",
///     || async {
///         let job = client.get_job(&job_id).await.unwrap();
///         job.task.status == Status::Running
///     },
/// ).await?;
/// ```
pub async fn wait_until<F, Fut>(
    timeout: Duration,
    poll_interval: Duration,
    description: &str,
    condition: F,
) -> Result<()>
where
    F: Fn() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if condition().await {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            bail!(
                "timed out after {:.1}s waiting for: {}",
                timeout.as_secs_f64(),
                description
            );
        }
        tokio::time::sleep(poll_interval).await;
    }
}
