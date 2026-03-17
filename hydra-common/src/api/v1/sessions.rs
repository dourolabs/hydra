use crate::{
    BuildCacheContext, IssueId, RepoName, SessionId, VersionNumber,
    actor_ref::ActorRef,
    task_status::{Status, TaskError},
    users::Username,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct Session {
    pub prompt: String,
    pub context: BundleSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawned_from: Option<IssueId>,
    pub creator: Username,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env_vars: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_limit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_limit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secrets: Option<Vec<String>>,
    #[serde(default = "default_status")]
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<TaskError>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creation_time: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_time: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_time: Option<DateTime<Utc>>,
}

impl Session {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        prompt: String,
        context: BundleSpec,
        spawned_from: Option<IssueId>,
        creator: Username,
        image: Option<String>,
        model: Option<String>,
        env_vars: HashMap<String, String>,
        cpu_limit: Option<String>,
        memory_limit: Option<String>,
        secrets: Option<Vec<String>>,
        status: Status,
        last_message: Option<String>,
        error: Option<TaskError>,
        deleted: bool,
        creation_time: Option<DateTime<Utc>>,
        start_time: Option<DateTime<Utc>>,
        end_time: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            prompt,
            context,
            spawned_from,
            creator,
            image,
            model,
            env_vars,
            cpu_limit,
            memory_limit,
            secrets,
            status,
            last_message,
            error,
            deleted,
            creation_time,
            start_time,
            end_time,
        }
    }
}

fn default_status() -> Status {
    Status::Created
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct CreateSessionRequest {
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default)]
    pub context: BundleSpec,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub variables: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_id: Option<IssueId>,
}

impl CreateSessionRequest {
    pub fn new(
        prompt: String,
        image: Option<String>,
        context: BundleSpec,
        variables: HashMap<String, String>,
        issue_id: Option<IssueId>,
    ) -> Self {
        Self {
            prompt,
            image,
            context,
            variables,
            issue_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum BundleSpec {
    #[serde(rename = "none")]
    None,
    GitRepository {
        /// Remote Git repository URL that should be cloned for the session context.
        url: String,
        /// Specific git revision (branch, tag, or commit) to checkout after cloning.
        rev: String,
    },
    ServiceRepository {
        /// Name of a repository configured in the service configuration.
        name: RepoName,
        /// Optional git revision (branch, tag, or commit) to checkout after cloning.
        #[serde(default)]
        rev: Option<String>,
    },
    #[serde(other)]
    Unknown,
}

impl Default for BundleSpec {
    fn default() -> Self {
        Self::None
    }
}

impl From<Bundle> for BundleSpec {
    fn from(bundle: Bundle) -> Self {
        match bundle {
            Bundle::None => BundleSpec::None,
            Bundle::GitRepository { url, rev } => BundleSpec::GitRepository { url, rev },
            Bundle::Unknown => BundleSpec::Unknown,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BundleSpecHelper {
    #[serde(rename = "none")]
    None,
    GitRepository {
        url: String,
        rev: String,
    },
    ServiceRepository {
        name: RepoName,
        #[serde(default)]
        rev: Option<String>,
    },
}

impl<'de> Deserialize<'de> for BundleSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        match serde_json::from_value::<BundleSpecHelper>(value) {
            Ok(BundleSpecHelper::None) => Ok(BundleSpec::None),
            Ok(BundleSpecHelper::GitRepository { url, rev }) => {
                Ok(BundleSpec::GitRepository { url, rev })
            }
            Ok(BundleSpecHelper::ServiceRepository { name, rev }) => {
                Ok(BundleSpec::ServiceRepository { name, rev })
            }
            Err(_) => Ok(BundleSpec::Unknown),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Bundle {
    #[serde(rename = "none")]
    None,
    GitRepository {
        /// Remote Git repository URL that should be cloned for the session context.
        url: String,
        /// Specific git revision (branch, tag, or commit) to checkout after cloning.
        rev: String,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BundleHelper {
    #[serde(rename = "none")]
    None,
    GitRepository {
        url: String,
        rev: String,
    },
}

impl<'de> Deserialize<'de> for Bundle {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        match serde_json::from_value::<BundleHelper>(value) {
            Ok(BundleHelper::None) => Ok(Bundle::None),
            Ok(BundleHelper::GitRepository { url, rev }) => Ok(Bundle::GitRepository { url, rev }),
            Err(_) => Ok(Bundle::Unknown),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct WorkerContext {
    pub request_context: Bundle,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub variables: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_cache: Option<BuildCacheContext>,
}

impl WorkerContext {
    pub fn new(
        request_context: Bundle,
        prompt: String,
        model: Option<String>,
        variables: HashMap<String, String>,
        build_cache: Option<BuildCacheContext>,
    ) -> Self {
        Self {
            request_context,
            prompt,
            model,
            variables,
            build_cache,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct CreateSessionResponse {
    #[serde(alias = "job_id")]
    pub session_id: SessionId,
}

impl CreateSessionResponse {
    pub fn new(session_id: SessionId) -> Self {
        Self { session_id }
    }
}

/// Lightweight summary of a session for list views.
///
/// Excludes `context`, `image`, `model`, `env_vars`, `cpu_limit`,
/// `memory_limit`, `secrets`, and `last_message`.
/// The `prompt` field is truncated to the first 20 characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct SessionSummary {
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawned_from: Option<IssueId>,
    pub creator: Username,
    #[serde(default = "default_status")]
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<TaskError>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creation_time: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_time: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_time: Option<DateTime<Utc>>,
}

impl From<&Session> for SessionSummary {
    fn from(session: &Session) -> Self {
        let prompt = if session.prompt.chars().count() > 20 {
            let mut s: String = session.prompt.chars().take(20).collect();
            s.push_str("...");
            s
        } else {
            session.prompt.clone()
        };
        let error = session.error.as_ref().map(|e| match e {
            TaskError::JobEngineError { reason } => {
                if reason.chars().count() > 100 {
                    let truncated: String = reason.chars().take(100).collect();
                    TaskError::JobEngineError {
                        reason: truncated + "...",
                    }
                } else {
                    e.clone()
                }
            }
            _ => e.clone(),
        });
        SessionSummary {
            prompt,
            spawned_from: session.spawned_from.clone(),
            creator: session.creator.clone(),
            status: session.status,
            error,
            deleted: session.deleted,
            creation_time: session.creation_time,
            start_time: session.start_time,
            end_time: session.end_time,
        }
    }
}

/// Summary-level version record for session list responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct SessionSummaryRecord {
    #[serde(alias = "job_id")]
    pub session_id: SessionId,
    pub version: VersionNumber,
    pub timestamp: DateTime<Utc>,
    #[serde(alias = "task")]
    pub session: SessionSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<ActorRef>,
}

impl From<&SessionVersionRecord> for SessionSummaryRecord {
    fn from(record: &SessionVersionRecord) -> Self {
        SessionSummaryRecord {
            session_id: record.session_id.clone(),
            version: record.version,
            timestamp: record.timestamp,
            session: SessionSummary::from(&record.session),
            actor: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ListSessionsResponse {
    #[serde(alias = "jobs")]
    pub sessions: Vec<SessionSummaryRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_count: Option<u64>,
}

impl ListSessionsResponse {
    pub fn new(sessions: Vec<SessionSummaryRecord>) -> Self {
        Self {
            sessions,
            next_cursor: None,
            total_count: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct SessionVersionRecord {
    #[serde(alias = "job_id")]
    pub session_id: SessionId,
    pub version: VersionNumber,
    pub timestamp: DateTime<Utc>,
    #[serde(alias = "task")]
    pub session: Session,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<ActorRef>,
}

impl SessionVersionRecord {
    pub fn new(
        session_id: SessionId,
        version: VersionNumber,
        timestamp: DateTime<Utc>,
        session: Session,
        actor: Option<ActorRef>,
    ) -> Self {
        Self {
            session_id,
            version,
            timestamp,
            session,
            actor,
        }
    }
}

use super::serde_helpers::{
    deserialize_comma_separated, deserialize_comma_separated_json, serialize_comma_separated,
    serialize_comma_separated_json,
};

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct SearchSessionsQuery {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawned_from: Option<IssueId>,
    /// Filter sessions spawned from any of these issue IDs (comma-separated, max 100).
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "serialize_comma_separated",
        deserialize_with = "deserialize_comma_separated"
    )]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub spawned_from_ids: Vec<IssueId>,
    #[serde(default)]
    pub include_deleted: Option<bool>,
    /// Filter sessions by status (comma-separated in query string). When multiple
    /// statuses are provided, a session matches if its status is any of the given values.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "serialize_comma_separated_json",
        deserialize_with = "deserialize_comma_separated_json"
    )]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub status: Vec<Status>,
    /// Maximum number of results to return. When omitted, all results are returned.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Opaque cursor from a previous response's `next_cursor` field.
    #[serde(default)]
    pub cursor: Option<String>,
    /// When true, include `total_count` in the response.
    #[serde(default)]
    pub count: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ListSessionVersionsResponse {
    pub versions: Vec<SessionVersionRecord>,
}

impl ListSessionVersionsResponse {
    pub fn new(versions: Vec<SessionVersionRecord>) -> Self {
        Self { versions }
    }
}

impl SearchSessionsQuery {
    pub fn new(
        q: Option<String>,
        spawned_from: Option<IssueId>,
        include_deleted: Option<bool>,
        status: Vec<Status>,
    ) -> Self {
        Self {
            q,
            spawned_from,
            spawned_from_ids: Vec::new(),
            include_deleted,
            status,
            limit: None,
            cursor: None,
            count: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct KillSessionResponse {
    #[serde(alias = "job_id")]
    pub session_id: SessionId,
    pub status: String,
}

impl KillSessionResponse {
    pub fn new(session_id: SessionId, status: String) -> Self {
        Self { session_id, status }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IssueId, test_helpers::serialize_query_params};
    use std::collections::HashMap;

    #[test]
    fn search_sessions_query_serializes_with_reqwest() {
        let issue_id = IssueId::new();
        let query = SearchSessionsQuery {
            q: Some("test query".to_string()),
            spawned_from: Some(issue_id.clone()),
            spawned_from_ids: vec![],
            include_deleted: None,
            status: vec![],
            limit: None,
            cursor: None,
            count: None,
        };

        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(params.get("q").map(String::as_str), Some("test query"));
        assert_eq!(
            params.get("spawned_from").map(String::as_str),
            Some(issue_id.as_ref())
        );
    }

    #[test]
    fn search_sessions_query_serializes_status_filter() {
        let query = SearchSessionsQuery::new(None, None, None, vec![Status::Running]);

        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(params.get("status").map(String::as_str), Some("running"));
    }

    #[test]
    fn search_sessions_query_serializes_multi_status_filter() {
        let query = SearchSessionsQuery::new(
            None,
            None,
            None,
            vec![Status::Created, Status::Pending, Status::Running],
        );

        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(
            params.get("status").map(String::as_str),
            Some("created,pending,running")
        );
    }

    #[test]
    fn search_sessions_query_deserializes_comma_separated_status() {
        let query: SearchSessionsQuery =
            serde_urlencoded::from_str("status=created%2Cpending%2Crunning").unwrap();
        assert_eq!(
            query.status,
            vec![Status::Created, Status::Pending, Status::Running]
        );
    }

    #[test]
    fn search_sessions_query_serializes_spawned_from_ids() {
        let id1 = IssueId::new();
        let id2 = IssueId::new();
        let query = SearchSessionsQuery::new(None, None, None, vec![]);
        let query = SearchSessionsQuery {
            spawned_from_ids: vec![id1.clone(), id2.clone()],
            ..query
        };

        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        let expected = format!("{id1},{id2}");
        assert_eq!(
            params.get("spawned_from_ids").map(String::as_str),
            Some(expected.as_str())
        );
    }

    #[test]
    fn search_sessions_query_deserializes_spawned_from_ids() {
        let query: SearchSessionsQuery =
            serde_urlencoded::from_str("spawned_from_ids=i-abcd%2Ci-efgh").unwrap();
        assert_eq!(query.spawned_from_ids.len(), 2);
        assert_eq!(query.spawned_from_ids[0].as_ref(), "i-abcd");
        assert_eq!(query.spawned_from_ids[1].as_ref(), "i-efgh");
    }

    #[test]
    fn search_sessions_query_serializes_empty_query() {
        let query = SearchSessionsQuery::default();

        let params = serialize_query_params(&query);
        assert!(
            params.is_empty(),
            "expected no query params for empty SearchSessionsQuery"
        );
    }

    fn make_test_session(prompt: &str) -> Session {
        Session::new(
            prompt.to_string(),
            BundleSpec::None,
            Some(IssueId::new()),
            Username::from("alice"),
            Some("worker:latest".to_string()),
            Some("claude-3".to_string()),
            HashMap::from([("KEY".to_string(), "val".to_string())]),
            Some("500m".to_string()),
            Some("1Gi".to_string()),
            Some(vec!["secret".to_string()]),
            Status::Running,
            Some("last message text".to_string()),
            None,
            false,
            Some(chrono::Utc::now()),
            Some(chrono::Utc::now()),
            None,
        )
    }

    #[test]
    fn session_summary_truncates_long_prompt() {
        let long_prompt = "x".repeat(500);
        let session = make_test_session(&long_prompt);
        let summary = SessionSummary::from(&session);
        assert_eq!(summary.prompt, format!("{}...", "x".repeat(20)));
    }

    #[test]
    fn session_summary_preserves_short_prompt() {
        let session = make_test_session("short prompt");
        let summary = SessionSummary::from(&session);
        assert_eq!(summary.prompt, "short prompt");
    }

    #[test]
    fn session_summary_excludes_heavy_fields() {
        let session = make_test_session("test prompt");
        let summary = SessionSummary::from(&session);
        let value = serde_json::to_value(&summary).unwrap();
        assert!(value.get("context").is_none());
        assert!(value.get("image").is_none());
        assert!(value.get("model").is_none());
        assert!(value.get("env_vars").is_none());
        assert!(value.get("cpu_limit").is_none());
        assert!(value.get("memory_limit").is_none());
        assert!(value.get("secrets").is_none());
        assert!(value.get("last_message").is_none());
    }

    #[test]
    fn session_summary_maps_all_fields() {
        let session = make_test_session("my prompt");
        let summary = SessionSummary::from(&session);
        assert_eq!(summary.prompt, "my prompt");
        assert!(summary.spawned_from.is_some());
        assert_eq!(summary.creator, Username::from("alice"));
        assert_eq!(summary.status, Status::Running);
        assert!(summary.error.is_none());
        assert!(!summary.deleted);
        assert!(summary.creation_time.is_some());
        assert!(summary.start_time.is_some());
        assert!(summary.end_time.is_none());
    }

    #[test]
    fn session_summary_record_from_version_record() {
        let session = make_test_session("record test");
        let session_id = crate::SessionId::new();
        let record =
            SessionVersionRecord::new(session_id.clone(), 7, chrono::Utc::now(), session, None);
        let summary_record = SessionSummaryRecord::from(&record);
        assert_eq!(summary_record.session_id, session_id);
        assert_eq!(summary_record.version, 7);
        assert_eq!(summary_record.session.prompt, "record test");
        assert_eq!(summary_record.actor, None);
    }

    #[test]
    fn session_summary_truncates_long_error_reason() {
        let long_reason = "e".repeat(200);
        let mut session = make_test_session("prompt");
        session.error = Some(TaskError::JobEngineError {
            reason: long_reason,
        });
        let summary = SessionSummary::from(&session);
        let error = summary.error.unwrap();
        match error {
            TaskError::JobEngineError { reason } => {
                assert_eq!(reason.chars().count(), 103);
                assert!(reason.ends_with("..."));
                assert_eq!(&reason[..100], &"e".repeat(100));
            }
            _ => panic!("expected JobEngineError"),
        }
    }

    #[test]
    fn session_summary_preserves_short_error_reason() {
        let short_reason = "something went wrong".to_string();
        let mut session = make_test_session("prompt");
        session.error = Some(TaskError::JobEngineError {
            reason: short_reason.clone(),
        });
        let summary = SessionSummary::from(&session);
        let error = summary.error.unwrap();
        match error {
            TaskError::JobEngineError { reason } => {
                assert_eq!(reason, short_reason);
            }
            _ => panic!("expected JobEngineError"),
        }
    }

    #[test]
    fn session_summary_record_omits_actor() {
        let session = make_test_session("actor test");
        let session_id = crate::SessionId::new();
        let actor = ActorRef::System {
            worker_name: "worker-1".to_string(),
            on_behalf_of: None,
        };
        let record =
            SessionVersionRecord::new(session_id, 1, chrono::Utc::now(), session, Some(actor));
        let summary_record = SessionSummaryRecord::from(&record);
        assert_eq!(summary_record.actor, None);
    }

    #[test]
    fn backward_compat_deserializes_job_id_field() {
        let session_id = crate::SessionId::new();
        let json = serde_json::json!({
            "job_id": session_id.to_string(),
            "status": "ok"
        });
        let resp: KillSessionResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.session_id, session_id);
    }
}
