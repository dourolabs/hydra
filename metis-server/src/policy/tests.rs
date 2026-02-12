use super::*;
use crate::app::event_bus::{MutationPayload, ServerEvent};
use crate::domain::issues::{Issue, IssueStatus, IssueType};
use crate::domain::users::Username;
use crate::policy::config::{PolicyConfig, PolicyEntry, PolicyList};
use crate::policy::context::{AutomationContext, Operation, OperationPayload, RestrictionContext};
use crate::policy::registry::PolicyRegistry;
use crate::store::MemoryStore;
use crate::test_utils;
use chrono::Utc;
use metis_common::IssueId;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

fn dummy_issue() -> Issue {
    Issue::new(
        IssueType::Task,
        "test".to_string(),
        Username::from("creator"),
        String::new(),
        IssueStatus::Open,
        None,
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
}

fn dummy_issue_payload() -> Arc<MutationPayload> {
    Arc::new(MutationPayload::Issue {
        old: None,
        new: dummy_issue(),
    })
}

fn dummy_patch_payload() -> Arc<MutationPayload> {
    Arc::new(MutationPayload::Patch {
        old: None,
        new: crate::domain::patches::Patch::new(
            "title".to_string(),
            "desc".to_string(),
            String::new(),
            crate::domain::patches::PatchStatus::Open,
            false,
            None,
            Vec::new(),
            metis_common::RepoName::new("test", "repo").unwrap(),
            None,
        ),
    })
}

fn dummy_document_payload() -> Arc<MutationPayload> {
    Arc::new(MutationPayload::Document {
        old: None,
        new: crate::domain::documents::Document {
            title: "test".to_string(),
            body_markdown: String::new(),
            path: None,
            created_by: None,
            deleted: false,
        },
    })
}

// ---------------------------------------------------------------------------
// Mock restriction that always allows
// ---------------------------------------------------------------------------
struct AllowAllRestriction;

#[async_trait]
impl Restriction for AllowAllRestriction {
    fn name(&self) -> &str {
        "allow_all"
    }

    async fn evaluate(&self, _ctx: &RestrictionContext<'_>) -> Result<(), PolicyViolation> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Mock restriction that always rejects
// ---------------------------------------------------------------------------
struct RejectRestriction {
    message: String,
}

impl RejectRestriction {
    fn new(message: &str) -> Self {
        Self {
            message: message.to_string(),
        }
    }
}

#[async_trait]
impl Restriction for RejectRestriction {
    fn name(&self) -> &str {
        "reject"
    }

    async fn evaluate(&self, _ctx: &RestrictionContext<'_>) -> Result<(), PolicyViolation> {
        Err(PolicyViolation {
            policy_name: "reject".to_string(),
            message: self.message.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// Mock automation that counts executions
// ---------------------------------------------------------------------------
struct CountingAutomation {
    count: Arc<AtomicUsize>,
    filter: EventFilter,
}

#[async_trait]
impl Automation for CountingAutomation {
    fn name(&self) -> &str {
        "counting"
    }

    fn event_filter(&self) -> EventFilter {
        self.filter.clone()
    }

    async fn execute(&self, _ctx: &AutomationContext<'_>) -> Result<(), AutomationError> {
        self.count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Mock automation that always fails
// ---------------------------------------------------------------------------
struct FailingAutomation;

#[async_trait]
impl Automation for FailingAutomation {
    fn name(&self) -> &str {
        "failing"
    }

    fn event_filter(&self) -> EventFilter {
        EventFilter::default()
    }

    async fn execute(&self, _ctx: &AutomationContext<'_>) -> Result<(), AutomationError> {
        Err(AutomationError::Other(anyhow::anyhow!("automation broke")))
    }
}

fn make_issue_payload() -> OperationPayload {
    use crate::domain::issues::{Issue, IssueStatus, IssueType};

    OperationPayload::Issue {
        issue_id: Some(IssueId::new()),
        new: Issue::new(
            IssueType::Task,
            "test issue".to_string(),
            Username::from("tester"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
        old: None,
    }
}

// ---------------------------------------------------------------------------
// PolicyEngine::check_restrictions tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn check_restrictions_passes_with_no_restrictions() {
    let engine = PolicyEngine::empty();
    let store = MemoryStore::new();
    let payload = make_issue_payload();
    let ctx = RestrictionContext {
        operation: Operation::CreateIssue,

        repo: None,
        payload: &payload,
        store: &store,
    };

    let result = engine.check_restrictions(&ctx).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn check_restrictions_passes_when_all_allow() {
    let engine = PolicyEngine::new(
        vec![Box::new(AllowAllRestriction), Box::new(AllowAllRestriction)],
        Vec::new(),
    );
    let store = MemoryStore::new();
    let payload = make_issue_payload();
    let ctx = RestrictionContext {
        operation: Operation::CreateIssue,

        repo: None,
        payload: &payload,
        store: &store,
    };

    let result = engine.check_restrictions(&ctx).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn check_restrictions_returns_first_violation() {
    let engine = PolicyEngine::new(
        vec![
            Box::new(AllowAllRestriction),
            Box::new(RejectRestriction::new(
                "Cannot close issue: 2 children still open.",
            )),
            Box::new(RejectRestriction::new("Should not reach this.")),
        ],
        Vec::new(),
    );
    let store = MemoryStore::new();
    let payload = make_issue_payload();
    let ctx = RestrictionContext {
        operation: Operation::UpdateIssue,

        repo: None,
        payload: &payload,
        store: &store,
    };

    let result = engine.check_restrictions(&ctx).await;
    assert!(result.is_err());
    let violation = result.unwrap_err();
    assert_eq!(violation.policy_name, "reject");
    assert!(violation.message.contains("2 children still open"));
}

#[tokio::test]
async fn policy_violation_has_descriptive_message() {
    let violation = PolicyViolation {
        policy_name: "issue_lifecycle_validation".to_string(),
        message: "Cannot close issue i-abc123: 2 child issues are still open (i-def456, i-ghi789). Close or drop all children first.".to_string(),
    };
    assert_eq!(violation.policy_name, "issue_lifecycle_validation");
    assert!(violation.message.contains("Cannot close issue"));
    assert!(
        violation
            .message
            .contains("Close or drop all children first")
    );

    let display = format!("{violation}");
    assert!(display.contains("[issue_lifecycle_validation]"));
}

// ---------------------------------------------------------------------------
// PolicyEngine::run_automations tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn run_automations_executes_matching_automations() {
    let count = Arc::new(AtomicUsize::new(0));
    let sentinel = ServerEvent::IssueCreated {
        seq: 0,
        issue_id: IssueId::new(),
        version: 0,
        timestamp: Utc::now(),
        payload: dummy_issue_payload(),
    };
    let engine = PolicyEngine::new(
        Vec::new(),
        vec![Box::new(CountingAutomation {
            count: count.clone(),
            filter: EventFilter {
                event_types: vec![mem::discriminant(&sentinel)],
            },
        })],
    );

    let event = ServerEvent::IssueCreated {
        seq: 1,
        issue_id: IssueId::new(),
        version: 1,
        timestamp: Utc::now(),
        payload: dummy_issue_payload(),
    };

    let handles = test_utils::test_state_handles();

    let ctx = AutomationContext {
        event: &event,
        app_state: &handles.state,
        store: handles.store.as_ref(),
    };

    engine.run_automations(&ctx).await;
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn run_automations_skips_non_matching_events() {
    let count = Arc::new(AtomicUsize::new(0));
    let sentinel = ServerEvent::PatchCreated {
        seq: 0,
        patch_id: metis_common::PatchId::new(),
        version: 0,
        timestamp: Utc::now(),
        payload: dummy_patch_payload(),
    };
    let engine = PolicyEngine::new(
        Vec::new(),
        vec![Box::new(CountingAutomation {
            count: count.clone(),
            filter: EventFilter {
                event_types: vec![mem::discriminant(&sentinel)],
            },
        })],
    );

    let event = ServerEvent::IssueCreated {
        seq: 1,
        issue_id: IssueId::new(),
        version: 1,
        timestamp: Utc::now(),
        payload: dummy_issue_payload(),
    };

    let handles = test_utils::test_state_handles();

    let ctx = AutomationContext {
        event: &event,
        app_state: &handles.state,
        store: handles.store.as_ref(),
    };

    engine.run_automations(&ctx).await;
    assert_eq!(count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn run_automations_logs_errors_but_continues() {
    let count = Arc::new(AtomicUsize::new(0));
    let engine = PolicyEngine::new(
        Vec::new(),
        vec![
            Box::new(FailingAutomation),
            Box::new(CountingAutomation {
                count: count.clone(),
                filter: EventFilter::default(),
            }),
        ],
    );

    let event = ServerEvent::IssueUpdated {
        seq: 2,
        issue_id: IssueId::new(),
        version: 3,
        timestamp: Utc::now(),
        payload: dummy_issue_payload(),
    };

    let handles = test_utils::test_state_handles();

    let ctx = AutomationContext {
        event: &event,
        app_state: &handles.state,
        store: handles.store.as_ref(),
    };

    engine.run_automations(&ctx).await;
    // The counting automation should still run even though the failing one errored.
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

// ---------------------------------------------------------------------------
// EventFilter tests
// ---------------------------------------------------------------------------

#[test]
fn event_filter_empty_matches_all() {
    let filter = EventFilter::default();
    let event = ServerEvent::DocumentDeleted {
        seq: 1,
        document_id: metis_common::DocumentId::new(),
        version: 1,
        timestamp: Utc::now(),
        payload: dummy_document_payload(),
    };
    assert!(filter.matches(&event));
}

#[test]
fn event_filter_specific_type_matches() {
    let sentinel = ServerEvent::PatchUpdated {
        seq: 0,
        patch_id: metis_common::PatchId::new(),
        version: 0,
        timestamp: Utc::now(),
        payload: dummy_patch_payload(),
    };
    let filter = EventFilter {
        event_types: vec![mem::discriminant(&sentinel)],
    };
    let matching = ServerEvent::PatchUpdated {
        seq: 1,
        patch_id: metis_common::PatchId::new(),
        version: 1,
        timestamp: Utc::now(),
        payload: dummy_patch_payload(),
    };
    let non_matching = ServerEvent::PatchCreated {
        seq: 2,
        patch_id: metis_common::PatchId::new(),
        version: 1,
        timestamp: Utc::now(),
        payload: dummy_patch_payload(),
    };
    assert!(filter.matches(&matching));
    assert!(!filter.matches(&non_matching));
}

// ---------------------------------------------------------------------------
// PolicyRegistry tests
// ---------------------------------------------------------------------------

#[test]
fn registry_build_with_valid_config() {
    let mut registry = PolicyRegistry::new();
    registry.register_restriction("test_restriction", |_params| {
        Ok(Box::new(AllowAllRestriction))
    });
    registry.register_automation("test_automation", |_params| {
        Ok(Box::new(CountingAutomation {
            count: Arc::new(AtomicUsize::new(0)),
            filter: EventFilter::default(),
        }))
    });

    let config = PolicyConfig {
        global: PolicyList {
            restrictions: vec![PolicyEntry::Name("test_restriction".to_string())],
            automations: vec![PolicyEntry::Name("test_automation".to_string())],
        },
        repos: Default::default(),
    };

    let engine = registry.build(&config);
    assert!(engine.is_ok());
    let engine = engine.unwrap();
    assert_eq!(engine.restriction_count(), 1);
    assert_eq!(engine.automation_count(), 1);
}

#[test]
fn registry_build_with_unknown_restriction_fails() {
    let registry = PolicyRegistry::new();

    let config = PolicyConfig {
        global: PolicyList {
            restrictions: vec![PolicyEntry::Name("nonexistent_policy".to_string())],
            automations: Vec::new(),
        },
        repos: Default::default(),
    };

    let result = registry.build(&config);
    let err = result.err().expect("should fail for unknown restriction");
    assert!(
        err.contains("unknown restriction policy: 'nonexistent_policy'"),
        "unexpected error: {err}"
    );
}

#[test]
fn registry_build_with_unknown_automation_fails() {
    let registry = PolicyRegistry::new();

    let config = PolicyConfig {
        global: PolicyList {
            restrictions: Vec::new(),
            automations: vec![PolicyEntry::Name("nonexistent_automation".to_string())],
        },
        repos: Default::default(),
    };

    let result = registry.build(&config);
    let err = result.err().expect("should fail for unknown automation");
    assert!(
        err.contains("unknown automation policy: 'nonexistent_automation'"),
        "unexpected error: {err}"
    );
}

#[test]
fn registry_build_empty_config_produces_empty_engine() {
    let registry = PolicyRegistry::new();
    let config = PolicyConfig::default();

    let engine = registry.build(&config).unwrap();
    assert_eq!(engine.restriction_count(), 0);
    assert_eq!(engine.automation_count(), 0);
}

#[test]
fn registry_build_with_params() {
    let mut registry = PolicyRegistry::new();
    registry.register_restriction("parameterized", |params| {
        // Verify the params are passed through
        let params = params.ok_or("expected params")?;
        let table = params.as_table().ok_or("expected table")?;
        if !table.contains_key("threshold") {
            return Err("missing 'threshold' param".to_string());
        }
        Ok(Box::new(AllowAllRestriction))
    });

    let config = PolicyConfig {
        global: PolicyList {
            restrictions: vec![PolicyEntry::WithParams {
                name: "parameterized".to_string(),
                params: {
                    let mut table = toml::map::Map::new();
                    table.insert("threshold".to_string(), toml::Value::Integer(5));
                    toml::Value::Table(table)
                },
            }],
            automations: Vec::new(),
        },
        repos: Default::default(),
    };

    let result = registry.build(&config);
    assert!(result.is_ok());
}

// ---------------------------------------------------------------------------
// PolicyConfig deserialization tests
// ---------------------------------------------------------------------------

#[test]
fn policy_config_deserializes_from_toml() {
    let toml_str = r#"
        restrictions = ["issue_lifecycle_validation", "task_state_machine"]
        automations = ["cascade_issue_status"]

        [repos."dourolabs/metis"]
        restrictions = ["issue_lifecycle_validation"]
        automations = []
    "#;

    let config: PolicyConfig = toml::from_str(toml_str).expect("should deserialize");
    assert_eq!(config.global.restrictions.len(), 2);
    assert_eq!(
        config.global.restrictions[0].name(),
        "issue_lifecycle_validation"
    );
    assert_eq!(config.global.restrictions[1].name(), "task_state_machine");
    assert_eq!(config.global.automations.len(), 1);
    assert_eq!(config.global.automations[0].name(), "cascade_issue_status");

    let repo_config = config
        .repos
        .get("dourolabs/metis")
        .expect("should have repo config");
    assert_eq!(repo_config.restrictions.len(), 1);
    assert!(repo_config.automations.is_empty());
}

#[test]
fn policy_config_deserializes_with_params() {
    let toml_str = r#"
        restrictions = []

        [[automations]]
        name = "cascade_issue_status"
        [automations.params]
        statuses = ["dropped", "failed"]
    "#;

    let config: PolicyConfig = toml::from_str(toml_str).expect("should deserialize");
    assert_eq!(config.global.automations.len(), 1);
    let entry = &config.global.automations[0];
    assert_eq!(entry.name(), "cascade_issue_status");
    let params = entry.params().expect("should have params");
    let table = params.as_table().expect("params should be a table");
    assert!(table.contains_key("statuses"));
}

#[test]
fn policy_config_default_is_empty() {
    let config = PolicyConfig::default();
    assert!(config.global.restrictions.is_empty());
    assert!(config.global.automations.is_empty());
    assert!(config.repos.is_empty());
}

// ---------------------------------------------------------------------------
// Shortcut method tests
// ---------------------------------------------------------------------------

fn make_dummy_document() -> crate::domain::documents::Document {
    crate::domain::documents::Document {
        title: "test".to_string(),
        body_markdown: String::new(),
        path: None,
        created_by: None,
        deleted: false,
    }
}

fn make_dummy_patch() -> crate::domain::patches::Patch {
    crate::domain::patches::Patch::new(
        "title".to_string(),
        "desc".to_string(),
        String::new(),
        crate::domain::patches::PatchStatus::Open,
        false,
        None,
        Vec::new(),
        metis_common::RepoName::new("test", "repo").unwrap(),
        None,
    )
}

fn make_dummy_task() -> crate::store::Task {
    crate::store::Task::new(
        "test prompt".to_string(),
        crate::domain::jobs::BundleSpec::None,
        None,
        None,
        None,
        Default::default(),
        None,
        None,
        None,
    )
}

#[tokio::test]
async fn check_create_issue_delegates_to_check_restrictions() {
    let engine = PolicyEngine::new(
        vec![Box::new(RejectRestriction::new("blocked"))],
        Vec::new(),
    );
    let store = MemoryStore::new();
    let issue = dummy_issue();

    let result = engine.check_create_issue(&issue, &store).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("blocked"));
}

#[tokio::test]
async fn check_create_issue_passes_when_allowed() {
    let engine = PolicyEngine::new(vec![Box::new(AllowAllRestriction)], Vec::new());
    let store = MemoryStore::new();
    let issue = dummy_issue();

    let result = engine.check_create_issue(&issue, &store).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn check_update_issue_delegates_to_check_restrictions() {
    let engine = PolicyEngine::new(
        vec![Box::new(RejectRestriction::new("blocked"))],
        Vec::new(),
    );
    let store = MemoryStore::new();
    let issue = dummy_issue();
    let issue_id = IssueId::new();

    let result = engine
        .check_update_issue(&issue_id, &issue, None, &store)
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn check_create_patch_delegates_to_check_restrictions() {
    let engine = PolicyEngine::new(
        vec![Box::new(RejectRestriction::new("blocked"))],
        Vec::new(),
    );
    let store = MemoryStore::new();
    let patch = make_dummy_patch();

    let result = engine.check_create_patch(&patch, &store).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn check_create_patch_passes_when_allowed() {
    let engine = PolicyEngine::new(vec![Box::new(AllowAllRestriction)], Vec::new());
    let store = MemoryStore::new();
    let patch = make_dummy_patch();

    let result = engine.check_create_patch(&patch, &store).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn check_create_document_delegates_to_check_restrictions() {
    let engine = PolicyEngine::new(
        vec![Box::new(RejectRestriction::new("blocked"))],
        Vec::new(),
    );
    let store = MemoryStore::new();
    let doc = make_dummy_document();

    let result = engine.check_create_document(&doc, &store).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn check_update_document_delegates_to_check_restrictions() {
    let engine = PolicyEngine::new(
        vec![Box::new(RejectRestriction::new("blocked"))],
        Vec::new(),
    );
    let store = MemoryStore::new();
    let doc = make_dummy_document();
    let doc_id = metis_common::DocumentId::new();

    let result = engine
        .check_update_document(&doc_id, &doc, None, &store)
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn check_update_job_delegates_to_check_restrictions() {
    let engine = PolicyEngine::new(
        vec![Box::new(RejectRestriction::new("blocked"))],
        Vec::new(),
    );
    let store = MemoryStore::new();
    let task = make_dummy_task();
    let task_id = metis_common::TaskId::new();

    let result = engine.check_update_job(&task_id, &task, None, &store).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn check_update_job_passes_when_allowed() {
    let engine = PolicyEngine::new(vec![Box::new(AllowAllRestriction)], Vec::new());
    let store = MemoryStore::new();
    let task = make_dummy_task();
    let task_id = metis_common::TaskId::new();

    let result = engine.check_update_job(&task_id, &task, None, &store).await;
    assert!(result.is_ok());
}
