use chrono::{DateTime, Utc};
use git2::Oid;
use metis_common::{PatchId, RepoName, TaskId};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::{fmt, str::FromStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PatchStatus {
    Open,
    Closed,
    Merged,
}

impl Default for PatchStatus {
    fn default() -> Self {
        Self::Open
    }
}

impl PatchStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            PatchStatus::Open => "open",
            PatchStatus::Closed => "closed",
            PatchStatus::Merged => "merged",
        }
    }
}

impl fmt::Display for PatchStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PatchStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let normalized = s.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "open" => Ok(PatchStatus::Open),
            "closed" => Ok(PatchStatus::Closed),
            "merged" => Ok(PatchStatus::Merged),
            other => Err(format!("unsupported patch status '{other}'")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    pub contents: String,
    pub is_approved: bool,
    pub author: String,
    /// Timestamp for when the review was recorded.
    #[serde(default)]
    pub submitted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubPr {
    pub owner: String,
    pub repo: String,
    pub number: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ci: Option<GithubCiStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GitOid(pub Oid);

impl Serialize for GitOid {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for GitOid {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        let oid = Oid::from_str(&value).map_err(de::Error::custom)?;
        Ok(Self(oid))
    }
}

impl FromStr for GitOid {
    type Err = git2::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Oid::from_str(s).map(Self)
    }
}

impl fmt::Display for GitOid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.to_string())
    }
}

impl From<Oid> for GitOid {
    fn from(value: Oid) -> Self {
        Self(value)
    }
}

impl From<GitOid> for Oid {
    fn from(value: GitOid) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Patch {
    #[serde(default)]
    pub title: String,
    pub description: String,
    pub diff: String,
    #[serde(default)]
    pub status: PatchStatus,
    /// True when the patch is an automatic backup created from a job's output after tool-use patch generation failed.
    #[serde(default)]
    pub is_automatic_backup: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<TaskId>,
    #[serde(default)]
    pub reviews: Vec<Review>,
    /// Name of the configured service repository this patch targets, when known.
    pub service_repo_name: RepoName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<GithubPr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchRecord {
    pub id: PatchId,
    pub patch: Patch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpsertPatchRequest {
    pub patch: Patch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpsertPatchResponse {
    pub patch_id: PatchId,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchPatchesQuery {
    #[serde(default)]
    pub q: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GithubCiState {
    Pending,
    Success,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubCiFailure {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubCiStatus {
    pub state: GithubCiState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<GithubCiFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListPatchesResponse {
    pub patches: Vec<PatchRecord>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;
    use std::collections::HashMap;

    fn serialize_query_params<T: Serialize>(value: &T) -> Vec<(String, String)> {
        let encoded = serde_urlencoded::to_string(value).unwrap();
        serde_urlencoded::from_str(&encoded).unwrap()
    }

    #[test]
    fn patch_status_from_str() {
        assert_eq!(PatchStatus::from_str("open").unwrap(), PatchStatus::Open);
        assert_eq!(
            PatchStatus::from_str("closed").unwrap(),
            PatchStatus::Closed
        );
        assert_eq!(
            PatchStatus::from_str("merged").unwrap(),
            PatchStatus::Merged
        );
        assert!(PatchStatus::from_str("invalid").is_err());
    }

    #[test]
    fn github_pr_serialization() {
        let pr = GithubPr {
            owner: "owner".to_string(),
            repo: "repo".to_string(),
            number: 123,
            head_ref: Some("head".to_string()),
            base_ref: Some("base".to_string()),
            url: Some("http://example.com".to_string()),
            ci: None,
        };

        let json = serde_json::to_string(&pr).unwrap();
        let deserialized: GithubPr = serde_json::from_str(&json).unwrap();
        assert_eq!(pr, deserialized);
    }

    #[test]
    fn patch_query_serializes_with_reqwest() {
        let query = SearchPatchesQuery {
            q: Some("my search".to_string()),
        };

        let params = serialize_query_params(&query);
        let params: HashMap<_, _> = params.into_iter().collect();

        assert_eq!(params.get("q").map(String::as_str), Some("my search"));
    }
}
