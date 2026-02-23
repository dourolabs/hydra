use super::users::Username;
use crate::{PatchId, RepoName, TaskId, VersionNumber, actor_ref::ActorRef};
use chrono::{DateTime, Utc};
use git2::Oid;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::{fmt, str::FromStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, type = "string"))]
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

/// A base–head SHA pair identifying the exact commit range a patch covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct CommitRange {
    pub base: GitOid,
    pub head: GitOid,
}

impl CommitRange {
    pub fn new(base: GitOid, head: GitOid) -> Self {
        Self { base, head }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
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
    /// The resolved username of the human/agent that authored the patch.
    pub creator: Username,
    #[serde(default)]
    pub reviews: Vec<Review>,
    /// Name of the configured service repository this patch targets, when known.
    pub service_repo_name: RepoName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<GithubPr>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
    /// The head branch name for this patch, independent of any GitHub PR.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_name: Option<String>,
    /// The base-to-head commit range this patch covers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_range: Option<CommitRange>,
    /// The target branch this patch is intended to be applied on top of.
    ///
    /// Note: `base_branch` may not be upstream of `branch_name` — the two
    /// branches share a common ancestor. The base branch may have received
    /// additional commits since work on the patch began.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub creation_timestamp: Option<DateTime<Utc>>,
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
        creator: Username,
        reviews: Vec<Review>,
        service_repo_name: RepoName,
        github: Option<GithubPr>,
        deleted: bool,
        branch_name: Option<String>,
        commit_range: Option<CommitRange>,
        base_branch: Option<String>,
    ) -> Self {
        Self {
            title,
            description,
            diff,
            status,
            is_automatic_backup,
            created_by,
            creator,
            reviews,
            service_repo_name,
            github,
            deleted,
            branch_name,
            commit_range,
            base_branch,
            creation_timestamp: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct PatchVersionRecord {
    pub patch_id: PatchId,
    pub version: VersionNumber,
    pub timestamp: DateTime<Utc>,
    pub patch: Patch,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<ActorRef>,
}

impl PatchVersionRecord {
    pub fn new(
        patch_id: PatchId,
        version: VersionNumber,
        timestamp: DateTime<Utc>,
        patch: Patch,
        actor: Option<ActorRef>,
    ) -> Self {
        Self {
            patch_id,
            version,
            timestamp,
            patch,
            actor,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct UpsertPatchResponse {
    pub patch_id: PatchId,
    pub version: VersionNumber,
}

impl UpsertPatchResponse {
    pub fn new(patch_id: PatchId, version: VersionNumber) -> Self {
        Self { patch_id, version }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct CreatePatchAssetQuery {
    #[serde(default)]
    pub name: Option<String>,
}

impl CreatePatchAssetQuery {
    pub fn new(name: Option<String>) -> Self {
        Self { name }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct CreatePatchAssetResponse {
    pub asset_url: String,
}

impl CreatePatchAssetResponse {
    pub fn new(asset_url: String) -> Self {
        Self { asset_url }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct SearchPatchesQuery {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub include_deleted: Option<bool>,
    /// Filter patches by status (e.g., Open, Closed). When multiple statuses
    /// are provided, a patch matches if its status is any of the given values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub status: Vec<PatchStatus>,
    /// Filter patches by exact branch name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_name: Option<String>,
}

impl SearchPatchesQuery {
    pub fn new(
        q: Option<String>,
        include_deleted: Option<bool>,
        status: Vec<PatchStatus>,
        branch_name: Option<String>,
    ) -> Self {
        Self {
            q,
            include_deleted,
            status,
            branch_name,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub enum GithubCiState {
    Pending,
    Success,
    Failed,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
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

/// Compact review information for patch list views.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ReviewSummary {
    pub count: u32,
    pub approved: bool,
}

impl ReviewSummary {
    pub fn from_reviews(reviews: &[Review]) -> Self {
        Self {
            count: reviews.len() as u32,
            approved: reviews.iter().any(|r| r.is_approved),
        }
    }
}

/// Lightweight summary of a patch for list views.
///
/// Excludes `diff`, `description`, `reviews[].contents`, and `commit_range`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct PatchSummary {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub status: PatchStatus,
    #[serde(default)]
    pub is_automatic_backup: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<TaskId>,
    pub creator: Username,
    pub review_summary: ReviewSummary,
    pub service_repo_name: RepoName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<GithubPr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
}

impl From<&Patch> for PatchSummary {
    fn from(patch: &Patch) -> Self {
        PatchSummary {
            title: patch.title.clone(),
            status: patch.status,
            is_automatic_backup: patch.is_automatic_backup,
            created_by: patch.created_by.clone(),
            creator: patch.creator.clone(),
            review_summary: ReviewSummary::from_reviews(&patch.reviews),
            service_repo_name: patch.service_repo_name.clone(),
            github: patch.github.clone(),
            branch_name: patch.branch_name.clone(),
            base_branch: patch.base_branch.clone(),
            deleted: patch.deleted,
        }
    }
}

/// Summary-level version record for patch list responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct PatchSummaryRecord {
    pub patch_id: PatchId,
    pub version: VersionNumber,
    pub timestamp: DateTime<Utc>,
    pub patch: PatchSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<ActorRef>,
}

impl From<&PatchVersionRecord> for PatchSummaryRecord {
    fn from(record: &PatchVersionRecord) -> Self {
        PatchSummaryRecord {
            patch_id: record.patch_id.clone(),
            version: record.version,
            timestamp: record.timestamp,
            patch: PatchSummary::from(&record.patch),
            actor: record.actor.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ListPatchesResponse {
    pub patches: Vec<PatchSummaryRecord>,
}

impl ListPatchesResponse {
    pub fn new(patches: Vec<PatchSummaryRecord>) -> Self {
        Self { patches }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ListPatchVersionsResponse {
    pub versions: Vec<PatchVersionRecord>,
}

impl ListPatchVersionsResponse {
    pub fn new(versions: Vec<PatchVersionRecord>) -> Self {
        Self { versions }
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
            include_deleted: None,
            status: Vec::new(),
            branch_name: None,
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

    #[test]
    fn patch_serde_round_trip_with_branch_and_commit_range() {
        let patch = Patch {
            title: "fix bug".to_string(),
            description: "a fix".to_string(),
            diff: "diff content".to_string(),
            status: PatchStatus::Open,
            is_automatic_backup: false,
            created_by: None,
            creator: Username::from("test-creator"),
            reviews: vec![],
            service_repo_name: "org/repo".parse().unwrap(),
            github: None,
            deleted: false,
            branch_name: Some("feature/my-branch".to_string()),
            commit_range: Some(CommitRange::new(
                "0000000000000000000000000000000000000001".parse().unwrap(),
                "0000000000000000000000000000000000000002".parse().unwrap(),
            )),
            base_branch: Some("main".to_string()),
            creation_timestamp: None,
        };

        let json = serde_json::to_string(&patch).unwrap();
        let deserialized: Patch = serde_json::from_str(&json).unwrap();
        assert_eq!(patch, deserialized);
        assert_eq!(
            deserialized.branch_name.as_deref(),
            Some("feature/my-branch")
        );
        assert!(deserialized.commit_range.is_some());
    }

    #[test]
    fn patch_deserializes_without_new_fields() {
        let json = r#"{
            "title": "old patch",
            "description": "desc",
            "diff": "",
            "status": "Open",
            "is_automatic_backup": false,
            "creator": "test-creator",
            "reviews": [],
            "service_repo_name": "org/repo"
        }"#;

        let patch: Patch = serde_json::from_str(json).unwrap();
        assert_eq!(patch.title, "old patch");
        assert_eq!(patch.branch_name, None);
        assert_eq!(patch.commit_range, None);
        assert_eq!(patch.base_branch, None);
    }

    #[test]
    fn commit_range_serde_round_trip() {
        let cr = CommitRange::new(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse().unwrap(),
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".parse().unwrap(),
        );
        let json = serde_json::to_string(&cr).unwrap();
        let deserialized: CommitRange = serde_json::from_str(&json).unwrap();
        assert_eq!(cr, deserialized);
    }

    fn make_test_patch() -> Patch {
        Patch {
            title: "fix bug".to_string(),
            description: "long description of the fix".to_string(),
            diff: "diff --git a/file.rs\n+added line\n".to_string(),
            status: PatchStatus::Open,
            is_automatic_backup: false,
            created_by: Some(crate::TaskId::new()),
            creator: Username::from("alice"),
            reviews: vec![
                Review::new("looks good".to_string(), true, "bob".to_string(), None),
                Review::new(
                    "needs changes".to_string(),
                    false,
                    "carol".to_string(),
                    None,
                ),
            ],
            service_repo_name: "org/repo".parse().unwrap(),
            github: None,
            deleted: false,
            branch_name: Some("feature/fix".to_string()),
            commit_range: Some(CommitRange::new(
                "0000000000000000000000000000000000000001".parse().unwrap(),
                "0000000000000000000000000000000000000002".parse().unwrap(),
            )),
            base_branch: Some("main".to_string()),
            creation_timestamp: None,
        }
    }

    #[test]
    fn review_summary_counts_reviews_and_checks_approval() {
        let reviews = vec![
            Review::new("ok".to_string(), false, "a".to_string(), None),
            Review::new("lgtm".to_string(), true, "b".to_string(), None),
        ];
        let summary = ReviewSummary::from_reviews(&reviews);
        assert_eq!(summary.count, 2);
        assert!(summary.approved);
    }

    #[test]
    fn review_summary_no_approval() {
        let reviews = vec![Review::new(
            "needs work".to_string(),
            false,
            "a".to_string(),
            None,
        )];
        let summary = ReviewSummary::from_reviews(&reviews);
        assert_eq!(summary.count, 1);
        assert!(!summary.approved);
    }

    #[test]
    fn review_summary_empty_reviews() {
        let summary = ReviewSummary::from_reviews(&[]);
        assert_eq!(summary.count, 0);
        assert!(!summary.approved);
    }

    #[test]
    fn patch_summary_excludes_diff_description_commit_range() {
        let patch = make_test_patch();
        let summary = PatchSummary::from(&patch);
        let value = serde_json::to_value(&summary).unwrap();
        assert!(value.get("diff").is_none());
        assert!(value.get("description").is_none());
        assert!(value.get("commit_range").is_none());
        assert!(value.get("reviews").is_none());
    }

    #[test]
    fn patch_summary_maps_all_fields() {
        let patch = make_test_patch();
        let summary = PatchSummary::from(&patch);
        assert_eq!(summary.title, "fix bug");
        assert_eq!(summary.status, PatchStatus::Open);
        assert!(!summary.is_automatic_backup);
        assert!(summary.created_by.is_some());
        assert_eq!(summary.creator, Username::from("alice"));
        assert_eq!(summary.review_summary.count, 2);
        assert!(summary.review_summary.approved);
        assert_eq!(summary.branch_name.as_deref(), Some("feature/fix"));
        assert_eq!(summary.base_branch.as_deref(), Some("main"));
        assert!(!summary.deleted);
    }

    #[test]
    fn patch_summary_record_from_version_record() {
        let patch = make_test_patch();
        let patch_id: PatchId = crate::PatchId::new();
        let record = PatchVersionRecord::new(patch_id.clone(), 5, chrono::Utc::now(), patch, None);
        let summary_record = PatchSummaryRecord::from(&record);
        assert_eq!(summary_record.patch_id, patch_id);
        assert_eq!(summary_record.version, 5);
        assert_eq!(summary_record.patch.title, "fix bug");
        assert_eq!(summary_record.actor, None);
    }
}
