use super::*;
use crate::app::event_bus::{EventType, MutationPayload, ServerEvent};
use crate::domain::issues::{Issue, IssueStatus, IssueType};
use crate::domain::users::Username;
use crate::policy::config::{PolicyConfig, PolicyEntry, PolicyList};
use crate::policy::context::{AutomationContext, Operation, OperationPayload, RestrictionContext};
use crate::policy::registry::{self, PolicyRegistry};
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
        actor: None,
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
        actor: None,
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
        actor: None,
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
    let engine = PolicyEngine::new(
        Vec::new(),
        vec![Box::new(CountingAutomation {
            count: count.clone(),
            filter: EventFilter {
                event_types: vec![EventType::IssueCreated],
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
    let engine = PolicyEngine::new(
        Vec::new(),
        vec![Box::new(CountingAutomation {
            count: count.clone(),
            filter: EventFilter {
                event_types: vec![EventType::PatchCreated],
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
    let filter = EventFilter {
        event_types: vec![EventType::PatchUpdated],
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
        ..Default::default()
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
        ..Default::default()
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
        ..Default::default()
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
        ..Default::default()
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

// ---------------------------------------------------------------------------
// Integration tests: config-driven policy engine
// ---------------------------------------------------------------------------

/// Test 1: Default config (no `[policies]` section) reproduces all current
/// behavior exactly — all 5 restrictions and 6 automations are active.
#[test]
fn default_config_enables_all_builtin_policies() {
    let registry = registry::build_default_registry();

    // Build engine with no PolicyConfig (simulates absent [policies] section)
    let engine = crate::app::AppState::build_policy_engine(None);

    assert_eq!(engine.restriction_count(), 5);
    assert_eq!(engine.automation_count(), 5);

    // Also verify that an explicit config listing all policies gives the same counts
    let all_config = PolicyConfig {
        global: PolicyList {
            restrictions: vec![
                PolicyEntry::Name("issue_lifecycle_validation".to_string()),
                PolicyEntry::Name("task_state_machine".to_string()),
                PolicyEntry::Name("duplicate_branch_name".to_string()),
                PolicyEntry::Name("running_job_validation".to_string()),
                PolicyEntry::Name("require_creator".to_string()),
            ],
            automations: vec![
                PolicyEntry::Name("cascade_issue_status".to_string()),
                PolicyEntry::Name("kill_tasks_on_issue_failure".to_string()),
                PolicyEntry::Name("close_merge_request_issues".to_string()),
                PolicyEntry::Name("create_merge_request_issue".to_string()),
                PolicyEntry::Name("github_pr_sync".to_string()),
            ],
        },
        ..Default::default()
    };
    let explicit_engine = registry.build(&all_config).unwrap();
    assert_eq!(explicit_engine.restriction_count(), 5);
    assert_eq!(explicit_engine.automation_count(), 5);
}

/// Test 2: Disabling a specific restriction allows the previously-blocked
/// operation. The `require_creator` restriction rejects issues with empty
/// creator fields. If we omit it from config, the operation should succeed.
#[tokio::test]
async fn disabling_restriction_allows_blocked_operation() {
    use crate::domain::issues::{Issue, IssueStatus, IssueType};

    // Engine with all restrictions including require_creator
    let full_engine = crate::app::AppState::build_policy_engine(None);
    let store = MemoryStore::new();

    let issue_no_creator = Issue::new(
        IssueType::Task,
        "test".to_string(),
        Username::from(""),
        String::new(),
        IssueStatus::Open,
        None,
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );

    // Full engine should block this (empty creator)
    let result = full_engine
        .check_create_issue(&issue_no_creator, &store)
        .await;
    assert!(
        result.is_err(),
        "full engine should block issue with empty creator"
    );

    // Build engine WITHOUT require_creator restriction
    let partial_config = PolicyConfig {
        global: PolicyList {
            restrictions: vec![
                PolicyEntry::Name("issue_lifecycle_validation".to_string()),
                PolicyEntry::Name("task_state_machine".to_string()),
                PolicyEntry::Name("duplicate_branch_name".to_string()),
                // require_creator is intentionally omitted
                PolicyEntry::Name("running_job_validation".to_string()),
            ],
            automations: vec![
                PolicyEntry::Name("cascade_issue_status".to_string()),
                PolicyEntry::Name("kill_tasks_on_issue_failure".to_string()),
                PolicyEntry::Name("close_merge_request_issues".to_string()),
                PolicyEntry::Name("create_merge_request_issue".to_string()),
                PolicyEntry::Name("github_pr_sync".to_string()),
            ],
        },
        ..Default::default()
    };

    let partial_engine = crate::app::AppState::build_policy_engine(Some(&partial_config));
    assert_eq!(partial_engine.restriction_count(), 4);

    // Partial engine should allow this
    let result = partial_engine
        .check_create_issue(&issue_no_creator, &store)
        .await;
    assert!(
        result.is_ok(),
        "engine without require_creator should allow issue with empty creator"
    );
}

/// Test 4: Parameterized policy works. The `cascade_issue_status` automation
/// accepts a `trigger_statuses` parameter that controls which issue statuses
/// trigger cascading. Verify it can be constructed with custom params.
#[test]
fn parameterized_policy_builds_with_custom_params() {
    let registry = registry::build_default_registry();

    // Config with cascade_issue_status using custom trigger_statuses
    let config = PolicyConfig {
        global: PolicyList {
            restrictions: vec![],
            automations: vec![PolicyEntry::WithParams {
                name: "cascade_issue_status".to_string(),
                params: {
                    let mut table = toml::map::Map::new();
                    let statuses = toml::Value::Array(vec![
                        toml::Value::String("dropped".to_string()),
                        toml::Value::String("failed".to_string()),
                    ]);
                    table.insert("trigger_statuses".to_string(), statuses);
                    toml::Value::Table(table)
                },
            }],
        },
        ..Default::default()
    };

    let engine = registry.build(&config);
    assert!(
        engine.is_ok(),
        "parameterized cascade_issue_status should build"
    );
    let engine = engine.unwrap();
    assert_eq!(engine.automation_count(), 1);
}

/// Test 5: Unknown policy name in config produces an error during validation.
#[test]
fn unknown_policy_name_in_config_errors() {
    let registry = registry::build_default_registry();

    // Config with an unknown restriction name
    let config = PolicyConfig {
        global: PolicyList {
            restrictions: vec![PolicyEntry::Name("nonexistent_restriction".to_string())],
            automations: vec![],
        },
        ..Default::default()
    };

    // Validation should error on unknown names
    let result = registry.validate_config(&config);
    assert!(
        result.is_err(),
        "validation should error on unknown policy names"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("unknown restriction policy 'nonexistent_restriction'"),
        "unexpected error: {err}"
    );

    // Building should also fail
    let build_result = registry.build(&config);
    assert!(
        build_result.is_err(),
        "build should fail for unknown policy names"
    );
}

/// Test: Invalid params for a known policy produce an error during validation.
#[test]
fn invalid_params_produce_error_during_validation() {
    let registry = registry::build_default_registry();

    // cascade_issue_status expects trigger_statuses to be an array, not a string
    let config = PolicyConfig {
        global: PolicyList {
            restrictions: vec![],
            automations: vec![PolicyEntry::WithParams {
                name: "cascade_issue_status".to_string(),
                params: toml::Value::String("invalid".to_string()),
            }],
        },
        ..Default::default()
    };

    let result = registry.validate_config(&config);
    assert!(result.is_err(), "validation should error on invalid params");
}

/// Test: TOML deserialization of a full config with policies section works.
#[test]
fn full_toml_config_with_policies_deserializes() {
    let toml_str = r#"
        [metis]
        namespace = "default"
        allowed_orgs = []

        [job]
        default_image = "metis-worker:latest"

        [database]
        url = "postgres://localhost/test"

        [github_app]
        app_id = 1
        client_id = "test"
        client_secret = "test"
        private_key = "test"

        [background]
        assignment_agent = "swe"

        [[background.agent_queues]]
        name = "swe"
        prompt = "test"

        [policies]
        restrictions = ["issue_lifecycle_validation", "task_state_machine"]
        automations = ["cascade_issue_status"]
    "#;

    let config: crate::config::AppConfig =
        toml::from_str(toml_str).expect("should deserialize full config with policies");

    let policies = config.policies.expect("policies should be present");
    assert_eq!(policies.global.restrictions.len(), 2);
    assert_eq!(policies.global.automations.len(), 1);
    assert_eq!(
        policies.global.restrictions[0].name(),
        "issue_lifecycle_validation"
    );
    assert_eq!(
        policies.global.automations[0].name(),
        "cascade_issue_status"
    );
}

/// Test: Config without [policies] section deserializes with policies = None.
#[test]
fn config_without_policies_deserializes_as_none() {
    let toml_str = r#"
        [metis]
        namespace = "default"

        [job]
        default_image = "metis-worker:latest"

        [database]
        url = "postgres://localhost/test"

        [github_app]
        app_id = 1
        client_id = "test"
        client_secret = "test"
        private_key = "test"

        [background]
        assignment_agent = "swe"

        [[background.agent_queues]]
        name = "swe"
        prompt = "test"
    "#;

    let config: crate::config::AppConfig =
        toml::from_str(toml_str).expect("should deserialize config without policies");
    assert!(
        config.policies.is_none(),
        "absent [policies] section should deserialize as None"
    );
}

// ---------------------------------------------------------------------------
// Per-repo policy config tests
// ---------------------------------------------------------------------------

#[test]
fn policy_config_deserializes_per_repo_overrides() {
    let toml_str = r#"
        restrictions = ["issue_lifecycle_validation"]
        automations = ["cascade_issue_status"]

        [repos."owner/repo"]
        restrictions = ["task_state_machine"]
        automations = ["kill_tasks_on_issue_failure"]
    "#;

    let config: PolicyConfig = toml::from_str(toml_str).expect("should deserialize");
    assert_eq!(config.global.restrictions.len(), 1);
    assert_eq!(
        config.global.restrictions[0].name(),
        "issue_lifecycle_validation"
    );
    assert_eq!(config.repos.len(), 1);
    let repo_list = config.repos.get("owner/repo").expect("repo should exist");
    assert_eq!(repo_list.restrictions.len(), 1);
    assert_eq!(repo_list.restrictions[0].name(), "task_state_machine");
    assert_eq!(repo_list.automations.len(), 1);
    assert_eq!(
        repo_list.automations[0].name(),
        "kill_tasks_on_issue_failure"
    );
}

#[test]
fn policy_config_per_repo_defaults_to_empty() {
    let toml_str = r#"
        restrictions = ["issue_lifecycle_validation"]
        automations = []
    "#;

    let config: PolicyConfig = toml::from_str(toml_str).expect("should deserialize");
    assert!(config.repos.is_empty());
}

#[test]
fn policy_config_multiple_repos() {
    let toml_str = r#"
        restrictions = []
        automations = []

        [repos."org/repo-a"]
        restrictions = ["task_state_machine"]
        automations = []

        [repos."org/repo-b"]
        restrictions = []
        automations = ["cascade_issue_status"]
    "#;

    let config: PolicyConfig = toml::from_str(toml_str).expect("should deserialize");
    assert_eq!(config.repos.len(), 2);
    assert_eq!(
        config.repos.get("org/repo-a").unwrap().restrictions.len(),
        1
    );
    assert_eq!(config.repos.get("org/repo-b").unwrap().automations.len(), 1);
}

#[test]
fn registry_builds_per_repo_engines() {
    let registry = registry::build_default_registry();

    let config = PolicyConfig {
        global: PolicyList {
            restrictions: vec![PolicyEntry::Name("issue_lifecycle_validation".to_string())],
            automations: vec![PolicyEntry::Name("cascade_issue_status".to_string())],
        },
        repos: {
            let mut m = std::collections::HashMap::new();
            m.insert(
                "owner/repo".to_string(),
                PolicyList {
                    restrictions: vec![
                        PolicyEntry::Name("task_state_machine".to_string()),
                        PolicyEntry::Name("require_creator".to_string()),
                    ],
                    automations: vec![],
                },
            );
            m
        },
    };

    let engine = registry.build(&config).expect("should build");
    // Global engine
    assert_eq!(engine.restriction_count(), 1);
    assert_eq!(engine.automation_count(), 1);
    // Per-repo engine count
    assert_eq!(engine.repo_engine_count(), 1);
}

#[test]
fn engine_for_repo_falls_back_to_global() {
    let registry = registry::build_default_registry();

    let config = PolicyConfig {
        global: PolicyList {
            restrictions: vec![PolicyEntry::Name("issue_lifecycle_validation".to_string())],
            automations: vec![PolicyEntry::Name("cascade_issue_status".to_string())],
        },
        repos: {
            let mut m = std::collections::HashMap::new();
            m.insert(
                "owner/repo".to_string(),
                PolicyList {
                    restrictions: vec![PolicyEntry::Name("task_state_machine".to_string())],
                    automations: vec![],
                },
            );
            m
        },
    };

    let engine = registry.build(&config).expect("should build");

    // Known repo uses per-repo engine
    let repo_engine = engine.engine_for_repo("owner/repo");
    assert_eq!(repo_engine.restrictions.len(), 1);
    assert_eq!(repo_engine.automations.len(), 0);

    // Unknown repo falls back to global
    let fallback = engine.engine_for_repo("other/repo");
    assert_eq!(fallback.restrictions.len(), 1);
    assert_eq!(fallback.automations.len(), 1);
}

#[test]
fn validate_config_catches_unknown_per_repo_policy() {
    let registry = registry::build_default_registry();

    let config = PolicyConfig {
        global: PolicyList {
            restrictions: vec![],
            automations: vec![],
        },
        repos: {
            let mut m = std::collections::HashMap::new();
            m.insert(
                "owner/repo".to_string(),
                PolicyList {
                    restrictions: vec![PolicyEntry::Name("nonexistent_restriction".to_string())],
                    automations: vec![],
                },
            );
            m
        },
    };

    let result = registry.validate_config(&config);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("unknown restriction policy 'nonexistent_restriction'"),
        "unexpected error: {err}"
    );
    assert!(
        err.contains("repos.owner/repo"),
        "error should mention repo scope: {err}"
    );
}

// ---------------------------------------------------------------------------
// PatchWorkflowConfig tests
// ---------------------------------------------------------------------------

#[test]
fn patch_workflow_config_deserializes_from_toml() {
    use crate::policy::config::PatchWorkflowConfig;

    let toml_str = r#"
        merge_request = { assignee = "$patch_creator" }
    "#;

    let config: PatchWorkflowConfig = toml::from_str(toml_str).expect("should deserialize");
    assert!(config.review_requests.is_empty());
    let mr = config
        .merge_request
        .expect("merge_request should be present");
    assert_eq!(mr.assignee, "$patch_creator");
}

#[test]
fn patch_workflow_config_with_review_requests() {
    use crate::policy::config::PatchWorkflowConfig;

    let toml_str = r#"
        review_requests = [
            { assignee = "reviewer1" },
            { assignee = "$patch_creator" },
        ]
        merge_request = { assignee = "swe" }
    "#;

    let config: PatchWorkflowConfig = toml::from_str(toml_str).expect("should deserialize");
    assert_eq!(config.review_requests.len(), 2);
    assert_eq!(config.review_requests[0].assignee, "reviewer1");
    assert_eq!(config.review_requests[1].assignee, "$patch_creator");
    let mr = config
        .merge_request
        .expect("merge_request should be present");
    assert_eq!(mr.assignee, "swe");
}

#[test]
fn patch_workflow_config_defaults_to_empty() {
    use crate::policy::config::PatchWorkflowConfig;

    let config = PatchWorkflowConfig::default();
    assert!(config.review_requests.is_empty());
    assert!(config.merge_request.is_none());
}

#[test]
fn patch_workflow_config_from_toml_params() {
    use crate::policy::config::PatchWorkflowConfig;

    // Simulate what happens when a TOML automation param is deserialized
    let toml_str = r#"
        review_requests = [
            { assignee = "jayantk" },
            { assignee = "$patch_creator" },
        ]
        merge_request = { assignee = "swe" }
    "#;

    let value: toml::Value = toml::from_str(toml_str).expect("should parse as TOML");
    let config: PatchWorkflowConfig = value
        .try_into()
        .expect("should deserialize into PatchWorkflowConfig");
    assert_eq!(config.review_requests.len(), 2);
    assert_eq!(config.review_requests[0].assignee, "jayantk");
    assert_eq!(config.review_requests[1].assignee, "$patch_creator");
    assert_eq!(config.merge_request.unwrap().assignee, "swe");
}

#[test]
fn patch_workflow_config_no_merge_request() {
    use crate::policy::config::PatchWorkflowConfig;

    let toml_str = r#"
        review_requests = [
            { assignee = "reviewer1" },
        ]
    "#;

    let config: PatchWorkflowConfig = toml::from_str(toml_str).expect("should deserialize");
    assert_eq!(config.review_requests.len(), 1);
    assert!(config.merge_request.is_none());
}

/// Full TOML config with per-repo policy overrides deserializes correctly.
#[test]
fn full_toml_config_with_per_repo_policies_deserializes() {
    let toml_str = r#"
        [metis]
        namespace = "default"
        allowed_orgs = []

        [job]
        default_image = "metis-worker:latest"

        [database]
        url = "postgres://localhost/test"

        [github_app]
        app_id = 1
        client_id = "test"
        client_secret = "test"
        private_key = "test"

        [background]
        assignment_agent = "swe"

        [[background.agent_queues]]
        name = "swe"
        prompt = "test"

        [policies]
        restrictions = ["issue_lifecycle_validation"]
        automations = ["cascade_issue_status"]

        [policies.repos."dourolabs/metis"]
        restrictions = ["task_state_machine", "require_creator"]
        automations = ["kill_tasks_on_issue_failure"]
    "#;

    let config: crate::config::AppConfig =
        toml::from_str(toml_str).expect("should deserialize full config with per-repo policies");

    let policies = config.policies.expect("policies should be present");
    assert_eq!(policies.global.restrictions.len(), 1);
    assert_eq!(policies.global.automations.len(), 1);
    assert_eq!(policies.repos.len(), 1);
    let repo = policies
        .repos
        .get("dourolabs/metis")
        .expect("repo should exist");
    assert_eq!(repo.restrictions.len(), 2);
    assert_eq!(repo.automations.len(), 1);
}
