use crate::RepoName;
use crate::api::v1::users::Username;
use crate::principal::Principal;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

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
#[cfg_attr(feature = "ts", ts(export, type = "\"@patch.creator\""))]
pub enum DynamicRef {
    /// The patch's creator. Only useful inside `mergers`.
    PatchCreator,
}

impl DynamicRef {
    /// The wire form *without* the leading `@`.
    pub fn shorthand(self) -> &'static str {
        match self {
            DynamicRef::PatchCreator => "patch.creator",
        }
    }

    /// Parse the part *after* the leading `@`. Returns a human-readable error
    /// listing the accepted values when the input does not match a known ref.
    ///
    /// Accepts `patch.author` as a deserialise-only alias for the canonical
    /// `patch.creator`. Existing stored merge_policy JSON blobs reference
    /// the old name; the lenient alias lets them continue to parse without a
    /// JSON-shape migration over the `repositories` table (same approach
    /// taken for the legacy bare-string form in [`parse_assignee_ref`]).
    pub fn from_shorthand(s: &str) -> Result<Self, String> {
        match s {
            "patch.creator" | "patch.author" => Ok(DynamicRef::PatchCreator),
            other => Err(format!(
                "unknown dynamic reference '@{other}'; expected one of @patch.creator"
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

/// A reference to a merge-policy principal — either a static
/// [`Principal`](crate::Principal) (user / agent / external) or a
/// [`DynamicRef`] resolved at merge-attempt time.
///
/// Phase 5a of `/designs/actor-system-overhaul.md` replaces this file's old
/// `Principal { User(Username), Dynamic(DynamicRef) }` enum with this
/// wrapper so the static side reuses the shared [`Principal`] (gaining
/// `Agent` / `External` variants, closing the "User-can-hide-agent"
/// footgun documented in §4.2).
///
/// **Wire form** is a single string, kept YAML-friendly:
///
/// - `"users/alice"`        → `Static(Principal::User { name })`
/// - `"agents/swe"`         → `Static(Principal::Agent { name })`
/// - `"external/github/x"`  → `Static(Principal::External { .. })`
/// - `"@patch.creator"`     → `Dynamic(DynamicRef::PatchCreator)`
///
/// For backwards compatibility with pre-Phase-5a configs (and existing
/// stored merge_policy JSON blobs), a bare username with no `/` or `@`
/// also deserialises as `Static(Principal::User { name })` — the
/// deserialiser is intentionally lenient so we do not need a JSON-blob
/// migration over the `repositories` table.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, type = "string"))]
pub enum AssigneeRef {
    Static(Principal),
    Dynamic(DynamicRef),
}

impl AssigneeRef {
    /// Construct a [`AssigneeRef::Static`] wrapping the given principal.
    pub fn static_principal(principal: Principal) -> Self {
        AssigneeRef::Static(principal)
    }

    /// Construct a [`AssigneeRef::Dynamic`] from a [`DynamicRef`].
    pub fn dynamic(dref: DynamicRef) -> Self {
        AssigneeRef::Dynamic(dref)
    }

    /// Canonical wire form: `users/<x>` / `agents/<x>` /
    /// `external/<sys>/<x>` for [`AssigneeRef::Static`] and
    /// `@<shorthand>` for [`AssigneeRef::Dynamic`].
    pub fn to_wire_string(&self) -> String {
        match self {
            AssigneeRef::Static(p) => p.to_path(),
            AssigneeRef::Dynamic(d) => format!("@{}", d.shorthand()),
        }
    }
}

impl fmt::Display for AssigneeRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_wire_string())
    }
}

impl Serialize for AssigneeRef {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_wire_string())
    }
}

impl<'de> Deserialize<'de> for AssigneeRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        parse_assignee_ref(&raw).map_err(D::Error::custom)
    }
}

/// Parse a single [`AssigneeRef`] wire string. Public for the SQL-side
/// equivalents to share a single oracle in tests.
pub fn parse_assignee_ref(raw: &str) -> Result<AssigneeRef, String> {
    if let Some(rest) = raw.strip_prefix('@') {
        let dr = DynamicRef::from_shorthand(rest)?;
        return Ok(AssigneeRef::Dynamic(dr));
    }
    // Path form: `users/...`, `agents/...`, `external/.../...`.
    if raw.contains('/') {
        return Principal::from_str(raw)
            .map(AssigneeRef::Static)
            .map_err(|e| e.to_string());
    }
    // Legacy bare-string fallback — treated as a User. This is the
    // pre-Phase-5a wire form; we keep accepting it so stored configs do
    // not need a JSON-blob migration.
    let name = Username::try_new(raw).map_err(|e| e.to_string())?;
    Ok(AssigneeRef::Static(Principal::User { name }))
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
    /// Principals — any combination of static `Principal`s and dynamic refs.
    pub any_of: Vec<AssigneeRef>,
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
    pub any_of: Vec<AssigneeRef>,
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

    /// Canonical form of a remote URL for equality matching.
    ///
    /// Rules (in order):
    ///   1. Trim whitespace.
    ///   2. Lowercase the host.
    ///   3. SSH form `git@HOST:OWNER/REPO[.git]` → `https://HOST/OWNER/REPO`.
    ///   4. Strip any `:PORT` from the host.
    ///   5. Strip trailing `.git`.
    ///   6. Strip trailing `/`.
    ///   7. Strip query string and fragment.
    ///   8. Leave `file://` and absolute-path remotes untouched
    ///      (already canonical).
    pub fn normalize_remote_url(raw: &str) -> String {
        let trimmed = raw.trim();

        // Rule 8: file:// and absolute-path remotes are already canonical.
        if trimmed.starts_with("file://") || trimmed.starts_with('/') {
            return trimmed.to_string();
        }

        // Rule 3: SCP-style SSH form `git@HOST:OWNER/REPO[.git]` →
        // `https://HOST/OWNER/REPO[.git]`. (`.git` is stripped by rule 5 below.)
        let converted = if let Some(rest) = trimmed.strip_prefix("git@")
            && let Some((host, path)) = rest.split_once(':')
        {
            format!("https://{host}/{path}")
        } else {
            trimmed.to_string()
        };

        // Rule 7: strip query string and fragment.
        let without_query = match converted.split_once('?') {
            Some((head, _)) => head.to_string(),
            None => converted,
        };
        let without_fragment = match without_query.split_once('#') {
            Some((head, _)) => head.to_string(),
            None => without_query,
        };

        // Split scheme:// from the rest. If there's no scheme, leave the input
        // alone (no host to lowercase or port to strip).
        let Some((scheme_with_sep, after_scheme)) = without_fragment.split_once("://") else {
            // Still apply rules 5 and 6 to whatever's left.
            let mut s = without_fragment;
            loop {
                let before = s.len();
                if let Some(stripped) = s.strip_suffix(".git") {
                    s = stripped.to_string();
                }
                while s.ends_with('/') {
                    s.pop();
                }
                if s.len() == before {
                    break;
                }
            }
            return s;
        };

        // Split host from path. The first `/` after the scheme separates them.
        let (host_part, path_part) = match after_scheme.split_once('/') {
            Some((h, p)) => (h.to_string(), format!("/{p}")),
            None => (after_scheme.to_string(), String::new()),
        };

        // Rules 2 + 4: lowercase host, strip `:PORT`.
        let host_no_port = match host_part.rfind(':') {
            Some(idx) => &host_part[..idx],
            None => host_part.as_str(),
        };
        let host = host_no_port.to_lowercase();

        let mut result = format!("{scheme_with_sep}://{host}{path_part}");

        // Rules 5 + 6: strip trailing `.git` and `/`. Run in a small fixed loop
        // so `…/repo.git/` collapses cleanly regardless of order.
        loop {
            let before = result.len();
            if let Some(stripped) = result.strip_suffix(".git") {
                result = stripped.to_string();
            }
            while result.ends_with('/') {
                result.pop();
            }
            if result.len() == before {
                break;
            }
        }

        result
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_deleted: Option<bool>,

    /// Filter to repositories whose `Repository.remote_url`, after canonical
    /// normalization via [`Repository::normalize_remote_url`], equals the
    /// normalized form of this value. Comparison is exact on the normalized
    /// strings; partial / substring / glob matches are not supported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_url: Option<String>,
}

impl SearchRepositoriesQuery {
    pub fn new(include_deleted: Option<bool>, remote_url: Option<String>) -> Self {
        Self {
            include_deleted,
            remote_url,
        }
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

    // ---- normalize_remote_url -------------------------------------------

    #[test]
    fn normalize_remote_url_github_https_with_git_suffix() {
        assert_eq!(
            Repository::normalize_remote_url("https://github.com/dourolabs/hydra.git"),
            "https://github.com/dourolabs/hydra"
        );
    }

    #[test]
    fn normalize_remote_url_github_https_without_git_suffix() {
        assert_eq!(
            Repository::normalize_remote_url("https://github.com/dourolabs/hydra"),
            "https://github.com/dourolabs/hydra"
        );
    }

    #[test]
    fn normalize_remote_url_github_ssh_with_git_suffix() {
        assert_eq!(
            Repository::normalize_remote_url("git@github.com:dourolabs/hydra.git"),
            "https://github.com/dourolabs/hydra"
        );
    }

    #[test]
    fn normalize_remote_url_github_ssh_without_git_suffix() {
        assert_eq!(
            Repository::normalize_remote_url("git@github.com:dourolabs/hydra"),
            "https://github.com/dourolabs/hydra"
        );
    }

    #[test]
    fn normalize_remote_url_trailing_slash() {
        assert_eq!(
            Repository::normalize_remote_url("https://github.com/dourolabs/hydra/"),
            "https://github.com/dourolabs/hydra"
        );
    }

    #[test]
    fn normalize_remote_url_uppercase_host() {
        assert_eq!(
            Repository::normalize_remote_url("https://GitHub.com/dourolabs/hydra"),
            "https://github.com/dourolabs/hydra"
        );
    }

    #[test]
    fn normalize_remote_url_strips_https_default_port() {
        assert_eq!(
            Repository::normalize_remote_url("https://github.com:443/dourolabs/hydra.git"),
            "https://github.com/dourolabs/hydra"
        );
    }

    #[test]
    fn normalize_remote_url_strips_ssh_default_port() {
        assert_eq!(
            Repository::normalize_remote_url("ssh://git@github.com:22/dourolabs/hydra.git"),
            "ssh://git@github.com/dourolabs/hydra"
        );
    }

    #[test]
    fn normalize_remote_url_strips_query_string() {
        assert_eq!(
            Repository::normalize_remote_url("https://github.com/dourolabs/hydra?ref=main"),
            "https://github.com/dourolabs/hydra"
        );
    }

    #[test]
    fn normalize_remote_url_strips_fragment() {
        assert_eq!(
            Repository::normalize_remote_url("https://github.com/dourolabs/hydra#readme"),
            "https://github.com/dourolabs/hydra"
        );
    }

    #[test]
    fn normalize_remote_url_strips_query_and_fragment() {
        assert_eq!(
            Repository::normalize_remote_url("https://github.com/dourolabs/hydra.git?ref=main#L42"),
            "https://github.com/dourolabs/hydra"
        );
    }

    #[test]
    fn normalize_remote_url_leaves_file_url_untouched() {
        assert_eq!(
            Repository::normalize_remote_url("file:///home/user/repo"),
            "file:///home/user/repo"
        );
    }

    #[test]
    fn normalize_remote_url_leaves_absolute_path_untouched() {
        assert_eq!(
            Repository::normalize_remote_url("/home/user/repo"),
            "/home/user/repo"
        );
    }

    #[test]
    fn normalize_remote_url_trims_whitespace() {
        assert_eq!(
            Repository::normalize_remote_url("  https://github.com/dourolabs/hydra.git  "),
            "https://github.com/dourolabs/hydra"
        );
    }

    #[test]
    fn normalize_remote_url_gitlab_https() {
        assert_eq!(
            Repository::normalize_remote_url("https://gitlab.com/group/subgroup/proj.git"),
            "https://gitlab.com/group/subgroup/proj"
        );
    }

    #[test]
    fn normalize_remote_url_bitbucket_https() {
        assert_eq!(
            Repository::normalize_remote_url("https://bitbucket.org/team/proj.git/"),
            "https://bitbucket.org/team/proj"
        );
    }

    #[test]
    fn normalize_remote_url_idempotent() {
        let canonical = "https://github.com/dourolabs/hydra";
        assert_eq!(Repository::normalize_remote_url(canonical), canonical);
    }

    #[test]
    fn normalize_remote_url_ssh_to_https_equivalence() {
        // Different surface forms of the same remote should normalize to the same string.
        let canonical = Repository::normalize_remote_url("https://github.com/dourolabs/hydra");
        for variant in [
            "https://github.com/dourolabs/hydra.git",
            "https://github.com/dourolabs/hydra/",
            "https://GitHub.com/dourolabs/hydra",
            "https://github.com:443/dourolabs/hydra",
            "git@github.com:dourolabs/hydra.git",
            "git@github.com:dourolabs/hydra",
            "  https://github.com/dourolabs/hydra.git  ",
            "https://github.com/dourolabs/hydra?ref=main",
            "https://github.com/dourolabs/hydra#readme",
        ] {
            assert_eq!(
                Repository::normalize_remote_url(variant),
                canonical,
                "variant `{variant}` should normalize to `{canonical}`"
            );
        }
    }

    #[test]
    fn search_repositories_query_remote_url_round_trips() {
        let q = SearchRepositoriesQuery::new(
            None,
            Some("https://github.com/dourolabs/hydra".to_string()),
        );
        let value = serde_json::to_value(&q).unwrap();
        let back: SearchRepositoriesQuery = serde_json::from_value(value).unwrap();
        assert_eq!(
            back.remote_url.as_deref(),
            Some("https://github.com/dourolabs/hydra")
        );
        assert_eq!(back.include_deleted, None);
    }

    #[test]
    fn search_repositories_query_remote_url_omitted_when_none() {
        let q = SearchRepositoriesQuery::new(None, None);
        let value = serde_json::to_value(&q).unwrap();
        assert!(
            !value.as_object().unwrap().contains_key("remote_url"),
            "remote_url should be omitted on the wire when None"
        );
    }

    // ---- MergePolicy ----------------------------------------------------

    use crate::api::v1::agents::AgentName;
    use crate::principal::ExternalSystem;

    fn user(name: &str) -> AssigneeRef {
        AssigneeRef::Static(Principal::User {
            name: Username::try_new(name).unwrap(),
        })
    }

    fn agent(name: &str) -> AssigneeRef {
        AssigneeRef::Static(Principal::Agent {
            name: AgentName::try_new(name).unwrap(),
        })
    }

    fn external(system: &str, username: &str) -> AssigneeRef {
        AssigneeRef::Static(Principal::External {
            system: ExternalSystem::try_new(system).unwrap(),
            username: username.to_string(),
        })
    }

    fn full_merge_policy() -> MergePolicy {
        MergePolicy {
            reviewers: vec![
                ReviewerGroup {
                    label: Some("code-review".to_string()),
                    any_of: vec![user("reviewer"), user("carol")],
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
                    AssigneeRef::Dynamic(DynamicRef::PatchCreator),
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
    fn merge_policy_round_trips_through_yaml_legacy_bare_strings() {
        // Mirrors the example in /designs/merge-time-constraints.md §4.2.
        // Uses the pre-Phase-5a bare-string wire form, which the lenient
        // deserialiser still accepts (treats every bare token as a `User`)
        // so existing stored merge_policy JSON blobs round-trip without a
        // JSON-shape migration.
        let yaml = r#"
reviewers:
  - label: code-review
    any_of:
      - reviewer
      - carol
    count: 1
    exclude_author: true
  - label: human-signoff
    any_of:
      - alice
      - bob
    count: 1

mergers:
  any_of:
    - "@patch.creator"
    - alice
"#;
        let policy: MergePolicy = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(policy.reviewers.len(), 2);
        assert_eq!(policy.reviewers[0].label.as_deref(), Some("code-review"));
        assert_eq!(
            policy.reviewers[0].any_of,
            vec![user("reviewer"), user("carol")]
        );
        assert_eq!(policy.reviewers[0].count, 1);
        assert!(policy.reviewers[0].exclude_author);
        assert_eq!(
            policy.mergers.as_ref().unwrap().any_of,
            vec![
                AssigneeRef::Dynamic(DynamicRef::PatchCreator),
                user("alice"),
            ]
        );

        // Re-serialising emits the new canonical path form
        // (`users/alice`), and re-parsing that form round-trips back to
        // the same value.
        let serialized = serde_yaml_ng::to_string(&policy).unwrap();
        assert!(
            serialized.contains("users/alice"),
            "re-serialisation should use the canonical path form, got: {serialized}"
        );
        let reparsed: MergePolicy = serde_yaml_ng::from_str(&serialized).unwrap();
        assert_eq!(reparsed, policy);
    }

    #[test]
    fn merge_policy_round_trips_through_yaml_path_form() {
        // Phase 5a canonical wire form: explicit path prefixes for every
        // static principal kind plus the existing `@patch.creator` shorthand
        // for dynamic refs.
        let yaml = r#"
reviewers:
  - any_of:
      - users/alice
      - agents/swe
      - external/github/jayantk
mergers:
  any_of:
    - "@patch.creator"
    - agents/swe
"#;
        let policy: MergePolicy = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(
            policy.reviewers[0].any_of,
            vec![user("alice"), agent("swe"), external("github", "jayantk"),]
        );
        assert_eq!(
            policy.mergers.as_ref().unwrap().any_of,
            vec![AssigneeRef::Dynamic(DynamicRef::PatchCreator), agent("swe"),]
        );
    }

    #[test]
    fn assignee_ref_static_serializes_in_canonical_path_form() {
        assert_eq!(
            serde_json::to_value(user("alice")).unwrap(),
            json!("users/alice")
        );
        assert_eq!(
            serde_json::to_value(agent("swe")).unwrap(),
            json!("agents/swe")
        );
        assert_eq!(
            serde_json::to_value(external("github", "jayantk")).unwrap(),
            json!("external/github/jayantk")
        );
    }

    #[test]
    fn assignee_ref_dynamic_serializes_with_at_prefix() {
        let value = serde_json::to_value(AssigneeRef::Dynamic(DynamicRef::PatchCreator)).unwrap();
        assert_eq!(value, json!("@patch.creator"));
    }

    #[test]
    fn assignee_ref_legacy_bare_string_deserializes_as_user() {
        // Backward-compat with pre-Phase-5a stored blobs.
        let back: AssigneeRef = serde_json::from_value(json!("alice")).unwrap();
        assert_eq!(back, user("alice"));
    }

    #[test]
    fn assignee_ref_path_form_deserializes_to_typed_principal() {
        let cases = [
            (json!("users/alice"), user("alice")),
            (json!("agents/swe"), agent("swe")),
            (
                json!("external/github/jayantk"),
                external("github", "jayantk"),
            ),
        ];
        for (wire, expected) in cases {
            let back: AssigneeRef = serde_json::from_value(wire.clone()).unwrap();
            assert_eq!(back, expected, "deserialize {wire}");
        }
    }

    #[test]
    fn dynamic_ref_shorthands_round_trip() {
        let variant = DynamicRef::PatchCreator;
        let wire = "@patch.creator";
        let principal = AssigneeRef::Dynamic(variant);
        let value = serde_json::to_value(&principal).unwrap();
        assert_eq!(value, json!(wire), "serialize {variant:?}");
        let back: AssigneeRef = serde_json::from_value(json!(wire)).unwrap();
        assert_eq!(back, principal, "deserialize {wire}");
    }

    #[test]
    fn assignee_ref_unknown_dynamic_ref_fails_with_useful_error() {
        let err = serde_json::from_value::<AssigneeRef>(json!("@nope.nope")).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("@nope.nope"),
            "error should name the offending ref, got: {msg}"
        );
        assert!(
            msg.contains("@patch.creator"),
            "error should list accepted refs, got: {msg}"
        );
    }

    #[test]
    fn dynamic_ref_legacy_patch_author_alias_deserializes_to_patch_creator() {
        // Stored merge_policy JSON blobs from before the rename use the
        // `@patch.author` literal. The lenient deserialiser maps them onto
        // the canonical `PatchCreator` variant so we do not need to migrate
        // the `repositories` table.
        let back: AssigneeRef = serde_json::from_value(json!("@patch.author")).unwrap();
        assert_eq!(back, AssigneeRef::Dynamic(DynamicRef::PatchCreator));
    }

    #[test]
    fn dynamic_ref_legacy_patch_author_alias_reserialises_as_patch_creator() {
        // Round-trip: stored `@patch.author` deserialises, then re-serialises
        // as the new canonical wire form `@patch.creator`.
        let back: AssigneeRef = serde_json::from_value(json!("@patch.author")).unwrap();
        let rewritten = serde_json::to_value(&back).unwrap();
        assert_eq!(rewritten, json!("@patch.creator"));
    }

    #[test]
    fn dynamic_ref_legacy_alias_equivalent_to_canonical_form() {
        // A policy parsed from the legacy alias is structurally equal to
        // one parsed from the canonical form.
        let from_alias: AssigneeRef = serde_json::from_value(json!("@patch.author")).unwrap();
        let from_canonical: AssigneeRef = serde_json::from_value(json!("@patch.creator")).unwrap();
        assert_eq!(from_alias, from_canonical);
    }

    #[test]
    fn removed_parent_issue_dynamic_refs_now_fail_validation() {
        // Once expressive options for the parent issue, these are no longer
        // accepted: policies that mention them must fail to parse with the
        // same error a typo would produce.
        for removed in ["@parent_issue.creator", "@parent_issue.assignee"] {
            let err = serde_json::from_value::<AssigneeRef>(json!(removed)).unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains(removed),
                "error should name the offending ref {removed}, got: {msg}"
            );
        }
    }

    #[test]
    fn assignee_ref_invalid_path_form_errors() {
        // `users/` with empty segment fails Principal::from_str.
        assert!(serde_json::from_value::<AssigneeRef>(json!("users/")).is_err());
        // Unknown prefix is reported as an unknown kind.
        let err = serde_json::from_value::<AssigneeRef>(json!("robots/r2")).unwrap_err();
        assert!(
            err.to_string().contains("robots"),
            "error should name the unknown kind, got: {err}",
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
        assert_eq!(obj.get("any_of"), Some(&json!(["users/alice"])));
    }

    #[test]
    fn legacy_stored_merge_policy_json_blob_parses() {
        // Snapshot of the pre-Phase-5a stored JSON shape (matches what
        // `hydra-web/packages/mock-server/fixtures/seed.json` ships for
        // `acme/api-gateway`). The lenient deserialiser must continue to
        // read it without a SQL-side migration, then re-serialise into
        // the canonical path form (and rename `@patch.author` to
        // `@patch.creator`).
        let legacy = json!({
            "reviewers": [
                {
                    "label": "code-review",
                    "any_of": ["reviewer", "carol"],
                    "count": 2
                },
                {
                    "label": "human-signoff",
                    "any_of": ["alice", "bob"]
                }
            ],
            "mergers": {"any_of": ["@patch.author", "alice"]}
        });
        let policy: MergePolicy = serde_json::from_value(legacy).unwrap();
        assert_eq!(
            policy.reviewers[0].any_of,
            vec![user("reviewer"), user("carol")]
        );
        assert_eq!(
            policy.mergers.as_ref().unwrap().any_of,
            vec![
                AssigneeRef::Dynamic(DynamicRef::PatchCreator),
                user("alice"),
            ]
        );
        // Reserialising migrates the bare strings to canonical path form
        // and the legacy `@patch.author` alias to `@patch.creator`.
        let rewritten = serde_json::to_value(&policy).unwrap();
        assert_eq!(
            rewritten["mergers"]["any_of"],
            json!(["@patch.creator", "users/alice"])
        );
        assert_eq!(
            rewritten["reviewers"][0]["any_of"],
            json!(["users/reviewer", "users/carol"])
        );
    }
}
