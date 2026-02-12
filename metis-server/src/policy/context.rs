use crate::app::AppState;
use crate::app::event_bus::ServerEvent;
use crate::store::Store;

/// The type of mutation being proposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    CreateIssue,
    UpdateIssue,
    DeleteIssue,
    CreatePatch,
    UpdatePatch,
    DeletePatch,
    CreateJob,
    UpdateJob,
    CreateDocument,
    UpdateDocument,
    DeleteDocument,
}

/// The payload describing the proposed change for restriction evaluation.
#[derive(Debug, Clone)]
pub enum OperationPayload {
    /// An issue is being created or updated.
    Issue {
        issue_id: Option<metis_common::IssueId>,
        new: crate::domain::issues::Issue,
        old: Option<crate::domain::issues::Issue>,
    },
    /// A patch is being created or updated.
    Patch {
        patch_id: Option<metis_common::PatchId>,
        new: crate::domain::patches::Patch,
        old: Option<crate::domain::patches::Patch>,
    },
    /// A job/task is being created or updated.
    Job {
        task_id: Option<metis_common::TaskId>,
        new: crate::store::Task,
        old: Option<crate::store::Task>,
    },
    /// A document is being created or updated.
    Document {
        document_id: Option<metis_common::DocumentId>,
        new: crate::domain::documents::Document,
        old: Option<crate::domain::documents::Document>,
    },
}

/// Context provided to restrictions for evaluating a proposed mutation.
pub struct RestrictionContext<'a> {
    pub operation: Operation,
    pub repo: Option<&'a metis_common::RepoName>,
    pub payload: &'a OperationPayload,
    pub store: &'a dyn Store,
}

/// Context provided to automations when an event fires.
pub struct AutomationContext<'a> {
    pub event: &'a ServerEvent,
    pub app_state: &'a AppState,
    pub store: &'a dyn Store,
}
