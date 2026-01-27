use crate::{MetisId, PatchId, RepoName, TaskId};
use chrono::{DateTime, Utc};
use git2::Oid;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::{fmt, str::FromStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum PatchStatus {
    Open,
    Closed,
    Merged,
    ChangesRequested,
    #[serde(other)]
    Unknown,
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
            PatchStatus::ChangesRequested => "changes-requested",
            PatchStatus::Unknown => "unknown",
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
            "changes-requested" | "changes_requested" | "changes requested" => {
                Ok(PatchStatus::ChangesRequested)
            }
            other => Err(format!("unsupported patch status '{other}'")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Review {
    pub contents: String,
    pub is_approved: bool,
    pub author: String,
    /// Timestamp for when the review was recorded.
    #[serde(default)]
    pub submitted_at: Option<DateTime<Utc>>,
}

impl Review {
    pub fn new(
        contents: String,
        is_approved: bool,
        author: String,
        submitted_at: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            contents,
            is_approved,
            author,
            submitted_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ReviewV2 {
    /// GitHub review id as provided by octocrab.
    pub review_id: MetisId,
    /// Login/handle of the reviewer who submitted the review.
    pub author: String,
    /// Review state from GitHub (e.g. "approved", "changes_requested", "commented").
    pub review_state: String,
    /// Timestamp for when the review was submitted, if known.
    pub submitted_at: Option<DateTime<Utc>>,
    /// Review body/message supplied with the review.
    pub review_message: Option<String>,
    /// Commit SHA the review is associated with, when GitHub provides it.
    pub commit_id: Option<String>,
    /// API URL for the review resource.
    pub url: Option<String>,
    /// HTML URL for the review in the GitHub UI.
    pub html_url: Option<String>,
    /// API URL for the pull request the review belongs to.
    pub pull_request_url: Option<String>,
    /// Review comments attached to this review submission.
    pub comments: Vec<ReviewCommentV2>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ReviewCommentV2 {
    /// GitHub review comment id as provided by octocrab.
    pub comment_id: MetisId,
    /// Review id that the comment belongs to.
    pub review_id: MetisId,
    /// Login/handle of the comment author, when known.
    pub author: Option<String>,
    /// Comment body text.
    pub body: String,
    /// API URL for the comment resource.
    pub url: Option<String>,
    /// HTML URL for the comment in the GitHub UI.
    pub html_url: Option<String>,
    /// File path in the diff that the comment refers to.
    pub path: String,
    /// Line number in the diff that the comment is attached to.
    pub line: Option<u32>,
    /// Original line number for the comment, if GitHub provides it.
    pub original_line: Option<u32>,
    /// Starting line number for multi-line comments, when present.
    pub start_line: Option<u32>,
    /// Original starting line number for multi-line comments, when present.
    pub original_start_line: Option<u32>,
    /// Side of the diff ("LEFT" or "RIGHT") for the selected line, when provided.
    pub side: Option<String>,
    /// Side of the diff ("LEFT" or "RIGHT") for the start line, when provided.
    pub start_side: Option<String>,
    /// Commit SHA the comment applies to, when GitHub provides it.
    pub commit_id: Option<String>,
    /// Original commit SHA for the comment, when GitHub provides it.
    pub original_commit_id: Option<String>,
    /// The diff hunk snippet included with the comment, when available.
    pub diff_hunk: Option<String>,
    /// Comment id this comment replies to, when present.
    pub in_reply_to: Option<MetisId>,
    /// Timestamp for when the comment was created, if known.
    pub created_at: Option<DateTime<Utc>>,
    /// Timestamp for when the comment was last updated, if known.
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
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

impl GithubPr {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        owner: String,
        repo: String,
        number: u64,
        head_ref: Option<String>,
        base_ref: Option<String>,
        url: Option<String>,
        ci: Option<GithubCiStatus>,
    ) -> Self {
        Self {
            owner,
            repo,
            number,
            head_ref,
            base_ref,
            url,
            ci,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct GitOid(pub Oid);

impl GitOid {
    pub fn new(oid: Oid) -> Self {
        Self(oid)
    }
}

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
#[non_exhaustive]
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

impl Patch {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        title: String,
        description: String,
        diff: String,
        status: PatchStatus,
        is_automatic_backup: bool,
        created_by: Option<TaskId>,
        reviews: Vec<Review>,
        service_repo_name: RepoName,
        github: Option<GithubPr>,
    ) -> Self {
        Self {
            title,
            description,
            diff,
            status,
            is_automatic_backup,
            created_by,
            reviews,
            service_repo_name,
            github,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PatchRecord {
    pub id: PatchId,
    pub patch: Patch,
}

impl PatchRecord {
    pub fn new(id: PatchId, patch: Patch) -> Self {
        Self { id, patch }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct UpsertPatchRequest {
    pub patch: Patch,
}

impl UpsertPatchRequest {
    pub fn new(patch: Patch) -> Self {
        Self { patch }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct UpsertPatchResponse {
    pub patch_id: PatchId,
}

impl UpsertPatchResponse {
    pub fn new(patch_id: PatchId) -> Self {
        Self { patch_id }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SearchPatchesQuery {
    #[serde(default)]
    pub q: Option<String>,
}

impl SearchPatchesQuery {
    pub fn new(q: Option<String>) -> Self {
        Self { q }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum GithubCiState {
    Pending,
    Success,
    Failed,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct GithubCiFailure {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details_url: Option<String>,
}

impl GithubCiFailure {
    pub fn new(name: String, summary: Option<String>, details_url: Option<String>) -> Self {
        Self {
            name,
            summary,
            details_url,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct GithubCiStatus {
    pub state: GithubCiState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<GithubCiFailure>,
}

impl GithubCiStatus {
    pub fn new(state: GithubCiState, failure: Option<GithubCiFailure>) -> Self {
        Self { state, failure }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ListPatchesResponse {
    pub patches: Vec<PatchRecord>,
}

impl ListPatchesResponse {
    pub fn new(patches: Vec<PatchRecord>) -> Self {
        Self { patches }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::serialize_query_params;
    use std::collections::HashMap;

    #[test]
    fn search_patches_query_serializes_with_reqwest() {
        let query = SearchPatchesQuery {
            q: Some("test query".to_string()),
        };

        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(params.get("q").map(String::as_str), Some("test query"));
    }

    #[test]
    fn search_patches_query_serializes_empty_query() {
        let query = SearchPatchesQuery::default();

        let params = serialize_query_params(&query);
        assert!(
            params.is_empty(),
            "expected empty SearchPatchesQuery to produce no parameters"
        );
    }

    #[test]
    fn patch_status_parses_from_strings() {
        assert_eq!(PatchStatus::from_str("open").unwrap(), PatchStatus::Open);
        assert_eq!(
            PatchStatus::from_str("CLOSED").unwrap(),
            PatchStatus::Closed
        );
        assert_eq!(
            PatchStatus::from_str(" merged ").unwrap(),
            PatchStatus::Merged
        );
        assert_eq!(
            PatchStatus::from_str("changes_requested").unwrap(),
            PatchStatus::ChangesRequested
        );
        assert!(PatchStatus::from_str("pending").is_err());
    }
}
