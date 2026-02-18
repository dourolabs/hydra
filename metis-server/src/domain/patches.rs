use super::users::Username;
use crate::domain::actors::UNKNOWN_CREATOR;
use chrono::{DateTime, Utc};
use git2::Oid;
use metis_common::api::v1 as api;
use metis_common::{RepoName, TaskId};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::{fmt, str::FromStr};

/// Serde default for backward compatibility with v1 JSONB payloads that lack the creator field.
fn default_patch_creator() -> Username {
    Username::from(UNKNOWN_CREATOR)
}

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

/// A base–head SHA pair identifying the exact commit range a patch covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Uses a serde default for backward compatibility with v1 JSONB payloads that lack the field.
    #[serde(default = "default_patch_creator")]
    pub creator: Username,
    #[serde(default)]
    pub reviews: Vec<Review>,
    /// Name of the configured service repository this patch targets, when known.
    pub service_repo_name: RepoName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<GithubPr>,
    #[serde(default)]
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
            deleted: false,
            branch_name: None,
            commit_range: None,
            base_branch: None,
        }
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
        Review {
            contents: value.contents,
            is_approved: value.is_approved,
            author: value.author,
            submitted_at: value.submitted_at,
        }
    }
}

impl From<Review> for api::patches::Review {
    fn from(value: Review) -> Self {
        api::patches::Review::new(
            value.contents,
            value.is_approved,
            value.author,
            value.submitted_at,
        )
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

impl From<api::patches::CommitRange> for CommitRange {
    fn from(value: api::patches::CommitRange) -> Self {
        CommitRange {
            base: value.base.into(),
            head: value.head.into(),
        }
    }
}

impl From<CommitRange> for api::patches::CommitRange {
    fn from(value: CommitRange) -> Self {
        api::patches::CommitRange::new(value.base.into(), value.head.into())
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
            creator: value.creator.into(),
            reviews: value.reviews.into_iter().map(Into::into).collect(),
            service_repo_name: value.service_repo_name,
            github: value.github.map(Into::into),
            deleted: value.deleted,
            branch_name: value.branch_name,
            commit_range: value.commit_range.map(Into::into),
            base_branch: value.base_branch,
        }
    }
}

impl From<Patch> for api::patches::Patch {
    fn from(value: Patch) -> Self {
        let mut patch = api::patches::Patch::new(
            value.title,
            value.description,
            value.diff,
            value.status.into(),
            value.is_automatic_backup,
            value.created_by,
            value.creator.into(),
            value.reviews.into_iter().map(Into::into).collect(),
            value.service_repo_name,
            value.github.map(Into::into),
            value.deleted,
        );
        patch.branch_name = value.branch_name;
        patch.commit_range = value.commit_range.map(Into::into);
        patch.base_branch = value.base_branch;
        patch
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

#[cfg(test)]
mod tests {
    use super::*;
    use metis_common::api::v1 as api;

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

    #[test]
    fn patch_round_trip_with_branch_and_commit_range() {
        let domain_patch = Patch {
            title: "test".to_string(),
            description: "desc".to_string(),
            diff: "diff".to_string(),
            status: PatchStatus::Open,
            is_automatic_backup: false,
            created_by: None,
            creator: Username::from("test-creator"),
            reviews: vec![],
            service_repo_name: "org/repo".parse().unwrap(),
            github: None,
            deleted: false,
            branch_name: Some("feature/branch".to_string()),
            commit_range: Some(CommitRange::new(
                "0000000000000000000000000000000000000001".parse().unwrap(),
                "0000000000000000000000000000000000000002".parse().unwrap(),
            )),
            base_branch: Some("main".to_string()),
        };

        let api_patch: api::patches::Patch = domain_patch.clone().into();
        assert_eq!(api_patch.branch_name.as_deref(), Some("feature/branch"));
        assert!(api_patch.commit_range.is_some());

        let round_trip: Patch = api_patch.into();
        assert_eq!(round_trip, domain_patch);
    }

    #[test]
    fn patch_domain_api_round_trip_without_new_fields() {
        let domain_patch = Patch::new(
            "title".to_string(),
            "desc".to_string(),
            "diff".to_string(),
            PatchStatus::Open,
            false,
            None,
            Username::from("test-creator"),
            vec![],
            "org/repo".parse().unwrap(),
            None,
        );

        assert_eq!(domain_patch.branch_name, None);
        assert_eq!(domain_patch.commit_range, None);

        let api_patch: api::patches::Patch = domain_patch.clone().into();
        let round_trip: Patch = api_patch.into();
        assert_eq!(round_trip, domain_patch);
    }
}
