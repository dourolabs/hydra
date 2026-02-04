use chrono::{DateTime, Utc};
use git2::Oid;
use metis_common::api::v1 as api;
use metis_common::{MetisId, PatchId, RepoName, TaskId};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::{fmt, str::FromStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PatchStatus {
    Open,
    Closed,
    Merged,
    ChangesRequested,
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
pub struct Review {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_id: Option<MetisId>,
    pub author: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_message: Option<String>,
    /// Timestamp for when the review was recorded.
    #[serde(default)]
    pub submitted_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub comments: Vec<ReviewComment>,
}

impl Review {
    pub fn new(
        contents: String,
        is_approved: bool,
        author: String,
        submitted_at: Option<DateTime<Utc>>,
    ) -> Self {
        let review_state = Some(if is_approved {
            "approved".to_string()
        } else {
            "commented".to_string()
        });
        Self {
            review_id: None,
            author,
            review_state,
            review_message: Some(contents),
            submitted_at,
            comments: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewComment {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment_id: Option<MetisId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_id: Option<MetisId>,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filepath: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_reply_to: Option<MetisId>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
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
pub struct UpsertPatchRequest {
    pub patch: Patch,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_github_branch: Option<String>,
}

impl UpsertPatchRequest {
    pub fn new(patch: Patch) -> Self {
        Self {
            patch,
            sync_github_branch: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpsertPatchResponse {
    pub patch_id: PatchId,
}

impl UpsertPatchResponse {
    pub fn new(patch_id: PatchId) -> Self {
        Self { patch_id }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
pub struct ListPatchesResponse {
    pub patches: Vec<PatchRecord>,
}

impl ListPatchesResponse {
    pub fn new(patches: Vec<PatchRecord>) -> Self {
        Self { patches }
    }
}

impl From<api::patches::PatchStatus> for PatchStatus {
    fn from(value: api::patches::PatchStatus) -> Self {
        match value {
            api::patches::PatchStatus::Open => PatchStatus::Open,
            api::patches::PatchStatus::Closed => PatchStatus::Closed,
            api::patches::PatchStatus::Merged => PatchStatus::Merged,
            api::patches::PatchStatus::ChangesRequested => PatchStatus::ChangesRequested,
            _ => unreachable!("unsupported PatchStatus variant"),
        }
    }
}

impl From<PatchStatus> for api::patches::PatchStatus {
    fn from(value: PatchStatus) -> Self {
        match value {
            PatchStatus::Open => api::patches::PatchStatus::Open,
            PatchStatus::Closed => api::patches::PatchStatus::Closed,
            PatchStatus::Merged => api::patches::PatchStatus::Merged,
            PatchStatus::ChangesRequested => api::patches::PatchStatus::ChangesRequested,
        }
    }
}

impl From<api::patches::Review> for Review {
    fn from(value: api::patches::Review) -> Self {
        let review_message = value.review_message.or(if value.contents.is_empty() {
            None
        } else {
            Some(value.contents)
        });
        let review_state = value.review_state.or_else(|| {
            if value.is_approved {
                Some("approved".to_string())
            } else if review_message.is_some() {
                Some("commented".to_string())
            } else {
                None
            }
        });

        Review {
            review_id: value.review_id,
            author: value.author,
            review_state,
            review_message,
            submitted_at: value.submitted_at,
            comments: value.comments.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<Review> for api::patches::Review {
    fn from(value: Review) -> Self {
        let is_approved = matches!(value.review_state.as_deref(), Some("approved"));
        let contents = value.review_message.clone().unwrap_or_default();
        let mut review =
            api::patches::Review::new(contents, is_approved, value.author, value.submitted_at);
        review.review_id = value.review_id;
        review.review_state = value.review_state;
        review.review_message = value.review_message;
        review.comments = value.comments.into_iter().map(Into::into).collect();
        review
    }
}

impl From<api::patches::ReviewComment> for ReviewComment {
    fn from(value: api::patches::ReviewComment) -> Self {
        ReviewComment {
            comment_id: value.comment_id,
            review_id: value.review_id,
            body: value.body,
            url: value.url,
            filepath: value.filepath,
            start_line: value.start_line,
            end_line: value.end_line,
            in_reply_to: value.in_reply_to,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

impl From<ReviewComment> for api::patches::ReviewComment {
    fn from(value: ReviewComment) -> Self {
        let mut comment = api::patches::ReviewComment::new(value.body);
        comment.comment_id = value.comment_id;
        comment.review_id = value.review_id;
        comment.url = value.url;
        comment.filepath = value.filepath;
        comment.start_line = value.start_line;
        comment.end_line = value.end_line;
        comment.in_reply_to = value.in_reply_to;
        comment.created_at = value.created_at;
        comment.updated_at = value.updated_at;
        comment
    }
}

impl From<api::patches::GithubPr> for GithubPr {
    fn from(value: api::patches::GithubPr) -> Self {
        GithubPr {
            owner: value.owner,
            repo: value.repo,
            number: value.number,
            head_ref: value.head_ref,
            base_ref: value.base_ref,
            url: value.url,
            ci: value.ci.map(Into::into),
        }
    }
}

impl From<GithubPr> for api::patches::GithubPr {
    fn from(value: GithubPr) -> Self {
        api::patches::GithubPr::new(
            value.owner,
            value.repo,
            value.number,
            value.head_ref,
            value.base_ref,
            value.url,
            value.ci.map(Into::into),
        )
    }
}

impl From<api::patches::GitOid> for GitOid {
    fn from(value: api::patches::GitOid) -> Self {
        GitOid(value.0)
    }
}

impl From<GitOid> for api::patches::GitOid {
    fn from(value: GitOid) -> Self {
        api::patches::GitOid::from(value.0)
    }
}

impl From<api::patches::Patch> for Patch {
    fn from(value: api::patches::Patch) -> Self {
        Patch {
            title: value.title,
            description: value.description,
            diff: value.diff,
            status: value.status.into(),
            is_automatic_backup: value.is_automatic_backup,
            created_by: value.created_by,
            reviews: value.reviews.into_iter().map(Into::into).collect(),
            service_repo_name: value.service_repo_name,
            github: value.github.map(Into::into),
        }
    }
}

impl From<Patch> for api::patches::Patch {
    fn from(value: Patch) -> Self {
        api::patches::Patch::new(
            value.title,
            value.description,
            value.diff,
            value.status.into(),
            value.is_automatic_backup,
            value.created_by,
            value.reviews.into_iter().map(Into::into).collect(),
            value.service_repo_name,
            value.github.map(Into::into),
        )
    }
}

impl From<api::patches::PatchRecord> for PatchRecord {
    fn from(value: api::patches::PatchRecord) -> Self {
        PatchRecord {
            id: value.id,
            patch: value.patch.into(),
        }
    }
}

impl From<PatchRecord> for api::patches::PatchRecord {
    fn from(value: PatchRecord) -> Self {
        api::patches::PatchRecord::new(value.id, value.patch.into())
    }
}

impl From<api::patches::UpsertPatchRequest> for UpsertPatchRequest {
    fn from(value: api::patches::UpsertPatchRequest) -> Self {
        UpsertPatchRequest {
            patch: value.patch.into(),
            sync_github_branch: value.sync_github_branch,
        }
    }
}

impl From<UpsertPatchRequest> for api::patches::UpsertPatchRequest {
    fn from(value: UpsertPatchRequest) -> Self {
        let upsert_request = api::patches::UpsertPatchRequest::new(value.patch.into());
        match value.sync_github_branch {
            Some(sync_github_branch) => upsert_request.with_sync_github_branch(&sync_github_branch),
            None => upsert_request,
        }
    }
}

impl From<api::patches::UpsertPatchResponse> for UpsertPatchResponse {
    fn from(value: api::patches::UpsertPatchResponse) -> Self {
        UpsertPatchResponse {
            patch_id: value.patch_id,
        }
    }
}

impl From<UpsertPatchResponse> for api::patches::UpsertPatchResponse {
    fn from(value: UpsertPatchResponse) -> Self {
        api::patches::UpsertPatchResponse::new(value.patch_id)
    }
}

impl From<api::patches::SearchPatchesQuery> for SearchPatchesQuery {
    fn from(value: api::patches::SearchPatchesQuery) -> Self {
        SearchPatchesQuery { q: value.q }
    }
}

impl From<SearchPatchesQuery> for api::patches::SearchPatchesQuery {
    fn from(value: SearchPatchesQuery) -> Self {
        api::patches::SearchPatchesQuery::new(value.q, None)
    }
}

impl From<api::patches::GithubCiState> for GithubCiState {
    fn from(value: api::patches::GithubCiState) -> Self {
        match value {
            api::patches::GithubCiState::Pending => GithubCiState::Pending,
            api::patches::GithubCiState::Success => GithubCiState::Success,
            api::patches::GithubCiState::Failed => GithubCiState::Failed,
            _ => unreachable!("unsupported GithubCiState variant"),
        }
    }
}

impl From<GithubCiState> for api::patches::GithubCiState {
    fn from(value: GithubCiState) -> Self {
        match value {
            GithubCiState::Pending => api::patches::GithubCiState::Pending,
            GithubCiState::Success => api::patches::GithubCiState::Success,
            GithubCiState::Failed => api::patches::GithubCiState::Failed,
        }
    }
}

impl From<api::patches::GithubCiFailure> for GithubCiFailure {
    fn from(value: api::patches::GithubCiFailure) -> Self {
        GithubCiFailure {
            name: value.name,
            summary: value.summary,
            details_url: value.details_url,
        }
    }
}

impl From<GithubCiFailure> for api::patches::GithubCiFailure {
    fn from(value: GithubCiFailure) -> Self {
        api::patches::GithubCiFailure::new(value.name, value.summary, value.details_url)
    }
}

impl From<api::patches::GithubCiStatus> for GithubCiStatus {
    fn from(value: api::patches::GithubCiStatus) -> Self {
        GithubCiStatus {
            state: value.state.into(),
            failure: value.failure.map(Into::into),
        }
    }
}

impl From<GithubCiStatus> for api::patches::GithubCiStatus {
    fn from(value: GithubCiStatus) -> Self {
        api::patches::GithubCiStatus::new(value.state.into(), value.failure.map(Into::into))
    }
}

impl From<api::patches::ListPatchesResponse> for ListPatchesResponse {
    fn from(value: api::patches::ListPatchesResponse) -> Self {
        ListPatchesResponse {
            patches: value.patches.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<ListPatchesResponse> for api::patches::ListPatchesResponse {
    fn from(value: ListPatchesResponse) -> Self {
        api::patches::ListPatchesResponse::new(value.patches.into_iter().map(Into::into).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use metis_common::api::v1 as api;
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
        assert_eq!(
            PatchStatus::from_str("changes requested").unwrap(),
            PatchStatus::ChangesRequested
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

    #[test]
    fn github_ci_status_converts_between_domain_and_api() {
        let domain = GithubCiStatus {
            state: GithubCiState::Failed,
            failure: Some(GithubCiFailure {
                name: "build".to_string(),
                summary: Some("compilation error".to_string()),
                details_url: None,
            }),
        };

        let api_value: api::patches::GithubCiStatus = domain.clone().into();
        let round_trip: GithubCiStatus = api_value.into();

        assert_eq!(round_trip, domain);
    }
}
