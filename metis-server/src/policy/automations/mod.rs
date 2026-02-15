pub mod cascade_issue_status;
pub mod close_merge_request_issues;
pub mod kill_tasks_on_failure;
pub mod patch_workflow;

pub use cascade_issue_status::CascadeIssueStatusAutomation;
pub use close_merge_request_issues::CloseMergeRequestIssuesAutomation;
pub use kill_tasks_on_failure::KillTasksOnFailureAutomation;
pub use patch_workflow::PatchWorkflowAutomation;
