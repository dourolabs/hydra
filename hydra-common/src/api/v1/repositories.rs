use crate::RepoName;
use crate::api::v1::users::Username;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

fn is_false(b: &bool) -> bool {
    !b
}

fn default_reviewer_count() -> u32 {
    1
}

fn is_default_reviewer_count(n: &u32) -> bool {
    *n == 1
}

fn default_exclude_author() -> bool {
    true
}

fn is_default_exclude_author(b: &bool) -> bool {
    *b
}

/// A closed enumeration of dynamic principal references resolvable at
/// merge-attempt time. Serialised as the `@…` shorthand string used in
/// repo-policy YAML and JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(
        export,
        type = "\"@parent_issue.creator\" | \"@parent_issue.assignee\" | \"@patch.author\""
    )
)]
pub enum DynamicRef {
    /// The user who created the patch's parent issue.
    ParentIssueCreator,
    /// The current assignee of the patch's parent issue.
    ParentIssueAssignee,
    /// The patch's author. Only useful inside `mergers`.
    PatchAuthor,
}

impl DynamicRef {
    /// The wire form *without* the leading `@`.
    pub fn shorthand(self) -> &'static str {
        match self {
            DynamicRef::ParentIssueCreator => "parent_issue.creator",
            DynamicRef::ParentIssueAssignee => "parent_issue.assignee",
            DynamicRef::PatchAuthor => "patch.author",
        }
    }

    /// Parse the part *after* the leading `@`. Returns a human-readable error
    /// listing the accepted values when the input does not match a known ref.
    pub fn from_shorthand(s: &str) -> Result<Self, String> {
        match s {
            "parent_issue.creator" => Ok(DynamicRef::ParentIssueCreator),
            "parent_issue.assignee" => Ok(DynamicRef::ParentIssueAssignee),
            "patch.author" => Ok(DynamicRef::PatchAuthor),
            other => Err(format!(
                "unknown dynamic reference '@{other}'; expected one of \
                 @parent_issue.creator, @parent_issue.assignee, @patch.author"
            )),
        }
    }
}

impl Serialize for DynamicRef {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut buf = String::with_capacity(self.shorthand().len() + 1);
        buf.push('@');
        buf.push_str(self.shorthand());
        serializer.serialize_str(&buf)
    }
}

impl<'de> Deserialize<'de> for DynamicRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        let rest = raw.strip_prefix('@').ok_or_else(|| {
            D::Error::custom(format!(
                "expected a dynamic reference starting with '@', got {raw:?}"
            ))
        })?;
        DynamicRef::from_shorthand(rest).map_err(D::Error::custom)
    }
}

/// A principal — either a static username or a dynamic reference resolved at
/// merge-attempt time.
///
/// Wire form is a single string: a bare username (e.g. `"alice"`) maps to
/// [`Principal::User`]; a string starting with `@` maps to
/// [`Principal::Dynamic`] against the closed [`DynamicRef`] enumeration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, type = "string"))]
pub enum Principal {
    User(Username),
    Dynamic(DynamicRef),
}

impl Serialize for Principal {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Principal::User(u) => serializer.serialize_str(u.as_str()),
            Principal::Dynamic(d) => d.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Principal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        if let Some(rest) = raw.strip_prefix('@') {
            let dr = DynamicRef::from_shorthand(rest).map_err(D::Error::custom)?;
            Ok(Principal::Dynamic(dr))
        } else {
            Ok(Principal::User(Username::from(raw)))
        }
    }
}

/// One reviewer group inside a [`MergePolicy`]. Members of `any_of` are
/// disjunctive; `count` distinct approving principals must be present (after
/// optional author exclusion) for the group to be satisfied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ReviewerGroup {
    /// Optional label surfaced in errors and in spawned review-request issues.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Principals — any combination of static usernames and dynamic refs.
    pub any_of: Vec<Principal>,
    /// Minimum number of distinct approving principals from `any_of` required
    /// for this group to be satisfied. Defaults to 1.
    #[serde(
        default = "default_reviewer_count",
        skip_serializing_if = "is_default_reviewer_count"
    )]
    pub count: u32,
    /// If true, the patch author is removed from the eligible set before
    /// counting. Defaults to true (matches GitHub behaviour).
    #[serde(
        default = "default_exclude_author",
        skip_serializing_if = "is_default_exclude_author"
    )]
    pub exclude_author: bool,
}

/// Who is permitted to call `hydra patches merge`. ANY match suffices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct MergerRule {
    pub any_of: Vec<Principal>,
}

/// Per-repository merge policy. ALL reviewer groups must be satisfied; the
/// acting actor must additionally match `mergers` if present.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct MergePolicy {
    #[serde(default)]
    pub reviewers: Vec<ReviewerGroup>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mergers: Option<MergerRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct Repository {
    pub remote_url: String,
    #[serde(default)]
    pub default_branch: Option<String>,
    #[serde(default)]
    pub default_image: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub deleted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_policy: Option<MergePolicy>,
}

impl Repository {
    pub fn new(
        remote_url: String,
        default_branch: Option<String>,
        default_image: Option<String>,
    ) -> Self {
        Self {
            remote_url,
            default_branch,
            default_image,
            deleted: false,
            merge_policy: None,
        }
    }

    /// Parse a GitHub remote URL to extract owner and repo name.
    ///
    /// Supports HTTPS (`https://github.com/owner/repo[.git]`) and
    /// SSH (`git@github.com:owner/repo[.git]`) formats.
    /// Returns `None` for non-GitHub URLs.
    ///
    /// All code that needs to determine whether a repository is hosted on GitHub
    /// (or extract owner/repo info) should use this method or [`is_github`](Self::is_github)
    /// rather than ad-hoc string matching on the remote URL.
    pub fn github_owner_repo(&self) -> Option<(String, String)> {
        let remote_url = &self.remote_url;

        // HTTPS: https://github.com/owner/repo.git or https://github.com/owner/repo
        if let Some(path) = remote_url
            .strip_prefix("https://github.com/")
            .or_else(|| remote_url.strip_prefix("http://github.com/"))
        {
            let path = path.trim_end_matches('/').trim_end_matches(".git");
            let (owner, repo) = path.split_once('/')?;
            if owner.is_empty() || repo.is_empty() || repo.contains('/') {
                return None;
            }
            return Some((owner.to_string(), repo.to_string()));
        }

        // SSH: git@github.com:owner/repo.git
        if let Some(path) = remote_url.strip_prefix("git@github.com:") {
            let path = path.trim_end_matches('/').trim_end_matches(".git");
            let (owner, repo) = path.split_once('/')?;
            if owner.is_empty() || repo.is_empty() || repo.contains('/') {
                return None;
            }
            return Some((owner.to_string(), repo.to_string()));
        }

        None
    }

    /// Returns `true` if this repository is hosted on GitHub.
    pub fn is_github(&self) -> bool {
        self.github_owner_repo().is_some()
    }

    /// Returns `true` if this repository is a local filesystem path.
    ///
    /// Detects `file://` URLs and absolute paths starting with `/`.
    pub fn is_local(&self) -> bool {
        self.remote_url.starts_with("file://") || self.remote_url.starts_with('/')
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct RepositoryRecord {
    pub name: RepoName,
    pub repository: Repository,
}

impl RepositoryRecord {
    pub fn new(name: RepoName, repository: Repository) -> Self {
        Self { name, repository }
    }
}

impl From<(RepoName, Repository)> for RepositoryRecord {
    fn from((name, repository): (RepoName, Repository)) -> Self {
        Self::new(name, repository)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct CreateRepositoryRequest {
    pub name: RepoName,
    #[serde(flatten)]
    pub repository: Repository,
}

impl CreateRepositoryRequest {
    pub fn new(name: RepoName, repository: Repository) -> Self {
        Self { name, repository }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct UpdateRepositoryRequest {
    #[serde(flatten)]
    pub repository: Repository,
}

impl UpdateRepositoryRequest {
    pub fn new(repository: Repository) -> Self {
        Self { repository }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct UpsertRepositoryResponse {
    pub repository: RepositoryRecord,
}

impl UpsertRepositoryResponse {
    pub fn new(repository: RepositoryRecord) -> Self {
        Self { repository }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct SearchRepositoriesQuery {
    #[serde(default)]
    pub include_deleted: Option<bool>,
}

impl SearchRepositoriesQuery {
    pub fn new(include_deleted: Option<bool>) -> Self {
        Self { include_deleted }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ListRepositoriesResponse {
    pub repositories: Vec<RepositoryRecord>,
}

impl ListRepositoriesResponse {
    pub fn new(repositories: Vec<RepositoryRecord>) -> Self {
        Self { repositories }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct DeleteRepositoryResponse {
    pub repository: RepositoryRecord,
}

impl DeleteRepositoryResponse {
    pub fn new(repository: RepositoryRecord) -> Self {
        Self { repository }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn github_owner_repo_https_with_git_suffix() {
        let repo = Repository::new(
            "https://github.com/dourolabs/hydra.git".to_string(),
            None,
            None,
        );
        assert_eq!(
            repo.github_owner_repo(),
            Some(("dourolabs".to_string(), "hydra".to_string()))
        );
    }

    #[test]
    fn github_owner_repo_https_without_git_suffix() {
        let repo = Repository::new("https://github.com/dourolabs/hydra".to_string(), None, None);
        assert_eq!(
            repo.github_owner_repo(),
            Some(("dourolabs".to_string(), "hydra".to_string()))
        );
    }

    #[test]
    fn github_owner_repo_ssh() {
        let repo = Repository::new("git@github.com:dourolabs/hydra.git".to_string(), None, None);
        assert_eq!(
            repo.github_owner_repo(),
            Some(("dourolabs".to_string(), "hydra".to_string()))
        );
    }

    #[test]
    fn github_owner_repo_ssh_without_git_suffix() {
        let repo = Repository::new("git@github.com:dourolabs/hydra".to_string(), None, None);
        assert_eq!(
            repo.github_owner_repo(),
            Some(("dourolabs".to_string(), "hydra".to_string()))
        );
    }

    #[test]
    fn github_owner_repo_non_github() {
        let repo = Repository::new("https://gitlab.com/org/repo.git".to_string(), None, None);
        assert_eq!(repo.github_owner_repo(), None);
    }

    #[test]
    fn github_owner_repo_file_url() {
        let repo = Repository::new("file:///home/user/repo".to_string(), None, None);
        assert_eq!(repo.github_owner_repo(), None);
    }

    #[test]
    fn github_owner_repo_empty_segments() {
        let repo = Repository::new("https://github.com//repo.git".to_string(), None, None);
        assert_eq!(repo.github_owner_repo(), None);

        let repo2 = Repository::new("https://github.com/owner/".to_string(), None, None);
        assert_eq!(repo2.github_owner_repo(), None);
    }

    #[test]
    fn is_github_returns_true_for_github_url() {
        let repo = Repository::new(
            "https://github.com/dourolabs/hydra.git".to_string(),
            None,
            None,
        );
        assert!(repo.is_github());
    }

    #[test]
    fn is_github_returns_false_for_non_github_url() {
        let repo = Repository::new("https://gitlab.com/org/repo.git".to_string(), None, None);
        assert!(!repo.is_github());
    }

    #[test]
    fn is_local_file_url() {
        let repo = Repository::new("file:///home/user/repo".to_string(), None, None);
        assert!(repo.is_local());
    }

    #[test]
    fn is_local_absolute_path() {
        let repo = Repository::new("/home/user/repo".to_string(), None, None);
        assert!(repo.is_local());
    }

    #[test]
    fn is_local_returns_false_for_github() {
        let repo = Repository::new(
            "https://github.com/dourolabs/hydra.git".to_string(),
            None,
            None,
        );
        assert!(!repo.is_local());
    }

    // ---- MergePolicy ----------------------------------------------------

    fn user(name: &str) -> Principal {
        Principal::User(Username::from(name))
    }

    fn full_merge_policy() -> MergePolicy {
        MergePolicy {
            reviewers: vec![
                ReviewerGroup {
                    label: Some("code-review".to_string()),
                    any_of: vec![
                        user("reviewer"),
                        Principal::Dynamic(DynamicRef::ParentIssueCreator),
                    ],
                    count: 2,
                    exclude_author: false,
                },
                ReviewerGroup {
                    label: Some("human-signoff".to_string()),
                    any_of: vec![user("alice"), user("bob")],
                    count: 1,
                    exclude_author: true,
                },
            ],
            mergers: Some(MergerRule {
                any_of: vec![
                    Principal::Dynamic(DynamicRef::ParentIssueCreator),
                    user("alice"),
                ],
            }),
        }
    }

    #[test]
    fn merge_policy_round_trips_through_serde_json() {
        let policy = full_merge_policy();
        let value = serde_json::to_value(&policy).unwrap();
        let back: MergePolicy = serde_json::from_value(value).unwrap();
        assert_eq!(back, policy);
    }

    #[test]
    fn merge_policy_round_trips_through_yaml() {
        // Mirrors the example in /designs/merge-time-constraints.md §4.2.
        let yaml = r#"
reviewers:
  - label: code-review
    any_of:
      - reviewer
      - "@parent_issue.creator"
    count: 1
    exclude_author: true
  - label: human-signoff
    any_of:
      - alice
      - bob
    count: 1

mergers:
  any_of:
    - "@parent_issue.creator"
    - alice
"#;
        let policy: MergePolicy = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(policy.reviewers.len(), 2);
        assert_eq!(policy.reviewers[0].label.as_deref(), Some("code-review"));
        assert_eq!(
            policy.reviewers[0].any_of,
            vec![
                user("reviewer"),
                Principal::Dynamic(DynamicRef::ParentIssueCreator),
            ]
        );
        assert_eq!(policy.reviewers[0].count, 1);
        assert!(policy.reviewers[0].exclude_author);
        assert_eq!(
            policy.mergers.as_ref().unwrap().any_of,
            vec![
                Principal::Dynamic(DynamicRef::ParentIssueCreator),
                user("alice"),
            ]
        );

        let serialized = serde_yaml_ng::to_string(&policy).unwrap();
        let reparsed: MergePolicy = serde_yaml_ng::from_str(&serialized).unwrap();
        assert_eq!(reparsed, policy);
    }

    #[test]
    fn principal_user_serializes_as_bare_string() {
        let value = serde_json::to_value(user("alice")).unwrap();
        assert_eq!(value, json!("alice"));
        let back: Principal = serde_json::from_value(json!("alice")).unwrap();
        assert_eq!(back, user("alice"));
    }

    #[test]
    fn dynamic_ref_shorthands_round_trip() {
        for (variant, wire) in [
            (DynamicRef::ParentIssueCreator, "@parent_issue.creator"),
            (DynamicRef::ParentIssueAssignee, "@parent_issue.assignee"),
            (DynamicRef::PatchAuthor, "@patch.author"),
        ] {
            let principal = Principal::Dynamic(variant);
            let value = serde_json::to_value(&principal).unwrap();
            assert_eq!(value, json!(wire), "serialize {variant:?}");
            let back: Principal = serde_json::from_value(json!(wire)).unwrap();
            assert_eq!(back, principal, "deserialize {wire}");
        }
    }

    #[test]
    fn principal_unknown_dynamic_ref_fails_with_useful_error() {
        let err = serde_json::from_value::<Principal>(json!("@nope.nope")).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("@nope.nope"),
            "error should name the offending ref, got: {msg}"
        );
        assert!(
            msg.contains("@parent_issue.creator"),
            "error should list accepted refs, got: {msg}"
        );
    }

    #[test]
    fn reviewer_group_defaults_apply_when_fields_missing() {
        let group: ReviewerGroup = serde_json::from_value(json!({
            "any_of": ["alice"],
        }))
        .unwrap();
        assert_eq!(group.label, None);
        assert_eq!(group.any_of, vec![user("alice")]);
        assert_eq!(group.count, 1, "missing count defaults to 1");
        assert!(
            group.exclude_author,
            "missing exclude_author defaults to true"
        );
    }

    #[test]
    fn merge_policy_missing_mergers_is_none() {
        let policy: MergePolicy = serde_json::from_value(json!({
            "reviewers": [{"any_of": ["alice"]}],
        }))
        .unwrap();
        assert!(policy.mergers.is_none());
    }

    #[test]
    fn repository_without_merge_policy_deserializes_to_none() {
        let json = json!({
            "remote_url": "https://example.com/repo.git",
        });
        let repo: Repository = serde_json::from_value(json).unwrap();
        assert!(repo.merge_policy.is_none());
    }

    #[test]
    fn repository_merge_policy_round_trips() {
        let mut repo = Repository::new("https://example.com/repo.git".to_string(), None, None);
        repo.merge_policy = Some(full_merge_policy());
        let value = serde_json::to_value(&repo).unwrap();
        let back: Repository = serde_json::from_value(value).unwrap();
        assert_eq!(back.merge_policy, Some(full_merge_policy()));
    }

    #[test]
    fn repository_merge_policy_none_is_omitted() {
        let repo = Repository::new("https://example.com/repo.git".to_string(), None, None);
        let value = serde_json::to_value(&repo).unwrap();
        assert!(
            !value.as_object().unwrap().contains_key("merge_policy"),
            "merge_policy should be omitted when None"
        );
    }

    #[test]
    fn reviewer_group_omits_defaults_when_serialized() {
        let group = ReviewerGroup {
            label: None,
            any_of: vec![user("alice")],
            count: 1,
            exclude_author: true,
        };
        let value = serde_json::to_value(&group).unwrap();
        let obj = value.as_object().unwrap();
        assert!(!obj.contains_key("label"));
        assert!(
            !obj.contains_key("count"),
            "default count should be omitted on the wire"
        );
        assert!(
            !obj.contains_key("exclude_author"),
            "default exclude_author should be omitted on the wire"
        );
        assert_eq!(obj.get("any_of"), Some(&json!(["alice"])));
    }
}
