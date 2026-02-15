pub mod cascade_issue_status;
pub mod close_merge_request_issues;
pub mod create_merge_request_issue;
pub mod kill_tasks_on_failure;

pub use cascade_issue_status::CascadeIssueStatusAutomation;
pub use close_merge_request_issues::CloseMergeRequestIssuesAutomation;
pub use create_merge_request_issue::CreateMergeRequestIssueAutomation;
pub use kill_tasks_on_failure::KillTasksOnFailureAutomation;
