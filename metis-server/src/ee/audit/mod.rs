use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The action that was performed on a resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    Created,
    Updated,
    Deleted,
}

impl std::fmt::Display for AuditAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuditAction::Created => f.write_str("created"),
            AuditAction::Updated => f.write_str("updated"),
            AuditAction::Deleted => f.write_str("deleted"),
        }
    }
}

impl std::str::FromStr for AuditAction {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "created" => Ok(AuditAction::Created),
            "updated" => Ok(AuditAction::Updated),
            "deleted" => Ok(AuditAction::Deleted),
            other => Err(format!("unsupported audit action '{other}'")),
        }
    }
}

/// The type of resource that was acted upon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditResourceType {
    Issue,
    Patch,
    Document,
    Actor,
    Label,
    Repository,
    Agent,
    User,
    Message,
}

impl std::fmt::Display for AuditResourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuditResourceType::Issue => f.write_str("issue"),
            AuditResourceType::Patch => f.write_str("patch"),
            AuditResourceType::Document => f.write_str("document"),
            AuditResourceType::Actor => f.write_str("actor"),
            AuditResourceType::Label => f.write_str("label"),
            AuditResourceType::Repository => f.write_str("repository"),
            AuditResourceType::Agent => f.write_str("agent"),
            AuditResourceType::User => f.write_str("user"),
            AuditResourceType::Message => f.write_str("message"),
        }
    }
}

impl std::str::FromStr for AuditResourceType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "issue" => Ok(AuditResourceType::Issue),
            "patch" => Ok(AuditResourceType::Patch),
            "document" => Ok(AuditResourceType::Document),
            "actor" => Ok(AuditResourceType::Actor),
            "label" => Ok(AuditResourceType::Label),
            "repository" => Ok(AuditResourceType::Repository),
            "agent" => Ok(AuditResourceType::Agent),
            "user" => Ok(AuditResourceType::User),
            "message" => Ok(AuditResourceType::Message),
            other => Err(format!("unsupported audit resource type '{other}'")),
        }
    }
}

/// An audit event recording a mutation performed in the system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub timestamp: DateTime<Utc>,
    pub actor_id: String,
    pub action: AuditAction,
    pub resource_type: AuditResourceType,
    pub resource_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}
