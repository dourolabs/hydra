pub mod cascade_issue_status;
pub mod close_merge_request_issues;
pub mod kill_tasks_on_failure;
pub mod notification_automation;
pub mod patch_workflow;
mod review_helpers;
pub mod sync_review_request_issues;

pub use cascade_issue_status::CascadeIssueStatusAutomation;
pub use close_merge_request_issues::CloseMergeRequestIssuesAutomation;
pub use kill_tasks_on_failure::KillTasksOnFailureAutomation;
pub use notification_automation::NotificationAutomation;
pub use patch_workflow::PatchWorkflowAutomation;
pub use sync_review_request_issues::SyncReviewRequestIssuesAutomation;
