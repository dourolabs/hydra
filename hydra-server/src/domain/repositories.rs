use super::users::Username;
use hydra_common::api::v1 as api;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashSet;
use std::fmt;
use thiserror::Error;

/// Domain mirror of [`api::repositories::DynamicRef`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DynamicRef {
    PatchAuthor,
}

impl DynamicRef {
    /// The wire form *without* the leading `@`.
    pub fn shorthand(self) -> &'static str {
        match self {
            DynamicRef::PatchAuthor => "patch.author",
        }
    }

    pub fn from_shorthand(s: &str) -> Result<Self, String> {
        match s {
            "patch.author" => Ok(DynamicRef::PatchAuthor),
            other => Err(format!(
                "unknown dynamic reference '@{other}'; expected one of @patch.author"
            )),
        }
    }
}

impl fmt::Display for DynamicRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@{}", self.shorthand())
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

/// Domain mirror of [`api::repositories::Principal`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Principal {
    User(Username),
    Dynamic(DynamicRef),
}

impl fmt::Display for Principal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Principal::User(name) => f.write_str(name.as_str()),
            Principal::Dynamic(d) => d.fmt(f),
        }
    }
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

/// Domain mirror of [`api::repositories::ReviewerGroup`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewerGroup {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub any_of: Vec<Principal>,
    pub count: u32,
    pub exclude_author: bool,
}

/// Domain mirror of [`api::repositories::MergerRule`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergerRule {
    pub any_of: Vec<Principal>,
}

/// Domain mirror of [`api::repositories::MergePolicy`].
///
/// Construct from an API value via [`MergePolicy::try_from`], which validates
/// the policy through [`validate_merge_policy`]. There is intentionally no
/// infallible API → domain conversion: the domain layer never holds an
/// unvalidated `MergePolicy`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergePolicy {
    #[serde(default)]
    pub reviewers: Vec<ReviewerGroup>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mergers: Option<MergerRule>,
}

/// Failure modes produced by [`validate_merge_policy`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MergePolicyValidationError {
    #[error("reviewer group {group_index} ({label}) has an empty `any_of`")]
    EmptyReviewerAnyOf { group_index: usize, label: String },

    #[error("`mergers.any_of` is empty; omit the `mergers` block instead")]
    EmptyMergerAnyOf,

    #[error("reviewer group {group_index} ({label}) has count = 0; count must be >= 1")]
    ZeroReviewerCount { group_index: usize, label: String },

    #[error(
        "reviewer group {group_index} ({label}) requires {count} approvals but only \
         {available} principals are eligible"
    )]
    ReviewerCountExceedsAnyOf {
        group_index: usize,
        label: String,
        count: u32,
        available: usize,
    },

    #[error("reviewer group label {label:?} appears on multiple groups; labels must be distinct")]
    DuplicateReviewerLabel { label: String },

    #[error(
        "invalid principal username {name:?}: usernames must be non-empty and contain no whitespace"
    )]
    InvalidUsername { name: String },

    #[error("reviewer group {group_index} ({label}) lists principal {principal} more than once")]
    DuplicatePrincipalInReviewerGroup {
        group_index: usize,
        label: String,
        principal: String,
    },

    #[error("`mergers.any_of` lists principal {principal} more than once")]
    DuplicatePrincipalInMergers { principal: String },
}

fn label_for_message(label: &Option<String>, group_index: usize) -> String {
    match label {
        Some(l) => format!("\"{l}\""),
        None => format!("#{group_index}"),
    }
}

fn validate_principal(p: &Principal) -> Result<(), MergePolicyValidationError> {
    if let Principal::User(name) = p {
        let s = name.as_str();
        if s.is_empty() || s.chars().any(char::is_whitespace) {
            return Err(MergePolicyValidationError::InvalidUsername {
                name: s.to_string(),
            });
        }
    }
    Ok(())
}

/// Validate a [`MergePolicy`] for structural and semantic correctness.
///
/// Returns the first failure encountered; callers are expected to surface the
/// error to the user rather than continue with a partially valid policy.
pub fn validate_merge_policy(policy: &MergePolicy) -> Result<(), MergePolicyValidationError> {
    let mut seen_labels: HashSet<&str> = HashSet::new();
    for (group_index, group) in policy.reviewers.iter().enumerate() {
        if let Some(label) = &group.label {
            if !seen_labels.insert(label.as_str()) {
                return Err(MergePolicyValidationError::DuplicateReviewerLabel {
                    label: label.clone(),
                });
            }
        }

        let label_msg = label_for_message(&group.label, group_index);

        if group.any_of.is_empty() {
            return Err(MergePolicyValidationError::EmptyReviewerAnyOf {
                group_index,
                label: label_msg,
            });
        }
        if group.count == 0 {
            return Err(MergePolicyValidationError::ZeroReviewerCount {
                group_index,
                label: label_msg,
            });
        }
        if (group.count as usize) > group.any_of.len() {
            return Err(MergePolicyValidationError::ReviewerCountExceedsAnyOf {
                group_index,
                label: label_msg,
                count: group.count,
                available: group.any_of.len(),
            });
        }

        let mut seen: HashSet<&Principal> = HashSet::new();
        for principal in &group.any_of {
            validate_principal(principal)?;
            if !seen.insert(principal) {
                return Err(
                    MergePolicyValidationError::DuplicatePrincipalInReviewerGroup {
                        group_index,
                        label: label_msg,
                        principal: principal.to_string(),
                    },
                );
            }
        }
    }

    if let Some(mergers) = &policy.mergers {
        if mergers.any_of.is_empty() {
            return Err(MergePolicyValidationError::EmptyMergerAnyOf);
        }
        let mut seen: HashSet<&Principal> = HashSet::new();
        for principal in &mergers.any_of {
            validate_principal(principal)?;
            if !seen.insert(principal) {
                return Err(MergePolicyValidationError::DuplicatePrincipalInMergers {
                    principal: principal.to_string(),
                });
            }
        }
    }

    Ok(())
}

// ---- API <-> domain conversions ------------------------------------------

impl From<api::repositories::DynamicRef> for DynamicRef {
    fn from(value: api::repositories::DynamicRef) -> Self {
        match value {
            api::repositories::DynamicRef::PatchAuthor => DynamicRef::PatchAuthor,
        }
    }
}

impl From<DynamicRef> for api::repositories::DynamicRef {
    fn from(value: DynamicRef) -> Self {
        match value {
            DynamicRef::PatchAuthor => api::repositories::DynamicRef::PatchAuthor,
        }
    }
}

impl From<api::repositories::Principal> for Principal {
    fn from(value: api::repositories::Principal) -> Self {
        match value {
            api::repositories::Principal::User(name) => Principal::User(name.into()),
            api::repositories::Principal::Dynamic(d) => Principal::Dynamic(d.into()),
        }
    }
}

impl From<Principal> for api::repositories::Principal {
    fn from(value: Principal) -> Self {
        match value {
            Principal::User(name) => api::repositories::Principal::User(name.into()),
            Principal::Dynamic(d) => api::repositories::Principal::Dynamic(d.into()),
        }
    }
}

impl From<api::repositories::ReviewerGroup> for ReviewerGroup {
    fn from(value: api::repositories::ReviewerGroup) -> Self {
        Self {
            label: value.label,
            any_of: value.any_of.into_iter().map(Into::into).collect(),
            count: value.count,
            exclude_author: value.exclude_author,
        }
    }
}

impl From<ReviewerGroup> for api::repositories::ReviewerGroup {
    fn from(value: ReviewerGroup) -> Self {
        Self {
            label: value.label,
            any_of: value.any_of.into_iter().map(Into::into).collect(),
            count: value.count,
            exclude_author: value.exclude_author,
        }
    }
}

impl From<api::repositories::MergerRule> for MergerRule {
    fn from(value: api::repositories::MergerRule) -> Self {
        Self {
            any_of: value.any_of.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<MergerRule> for api::repositories::MergerRule {
    fn from(value: MergerRule) -> Self {
        Self {
            any_of: value.any_of.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<MergePolicy> for api::repositories::MergePolicy {
    fn from(value: MergePolicy) -> Self {
        Self {
            reviewers: value.reviewers.into_iter().map(Into::into).collect(),
            mergers: value.mergers.map(Into::into),
        }
    }
}

/// Validating conversion: the canonical API → domain entry point for the
/// merge policy. The domain layer never holds an unvalidated [`MergePolicy`],
/// so there is no infallible `From<api::MergePolicy> for MergePolicy` impl.
impl TryFrom<api::repositories::MergePolicy> for MergePolicy {
    type Error = MergePolicyValidationError;

    fn try_from(value: api::repositories::MergePolicy) -> Result<Self, Self::Error> {
        let domain = MergePolicy {
            reviewers: value.reviewers.into_iter().map(Into::into).collect(),
            mergers: value.mergers.map(Into::into),
        };
        validate_merge_policy(&domain)?;
        Ok(domain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hydra_common::api::v1::repositories as api_repos;
    use hydra_common::api::v1::users::Username as ApiUsername;

    fn user(name: &str) -> Principal {
        Principal::User(Username::from(name))
    }

    fn api_user(name: &str) -> api_repos::Principal {
        api_repos::Principal::User(ApiUsername::from(name))
    }

    fn valid_policy() -> MergePolicy {
        MergePolicy {
            reviewers: vec![
                ReviewerGroup {
                    label: Some("code-review".to_string()),
                    any_of: vec![user("reviewer"), user("carol")],
                    count: 1,
                    exclude_author: true,
                },
                ReviewerGroup {
                    label: Some("human-signoff".to_string()),
                    any_of: vec![user("alice"), user("bob")],
                    count: 2,
                    exclude_author: false,
                },
            ],
            mergers: Some(MergerRule {
                any_of: vec![Principal::Dynamic(DynamicRef::PatchAuthor), user("alice")],
            }),
        }
    }

    fn api_policy_matching_valid_policy() -> api_repos::MergePolicy {
        api_repos::MergePolicy {
            reviewers: vec![
                api_repos::ReviewerGroup {
                    label: Some("code-review".to_string()),
                    any_of: vec![api_user("reviewer"), api_user("carol")],
                    count: 1,
                    exclude_author: true,
                },
                api_repos::ReviewerGroup {
                    label: Some("human-signoff".to_string()),
                    any_of: vec![api_user("alice"), api_user("bob")],
                    count: 2,
                    exclude_author: false,
                },
            ],
            mergers: Some(api_repos::MergerRule {
                any_of: vec![
                    api_repos::Principal::Dynamic(api_repos::DynamicRef::PatchAuthor),
                    api_user("alice"),
                ],
            }),
        }
    }

    #[test]
    fn api_to_domain_and_back_round_trips() {
        let api_policy = api_policy_matching_valid_policy();
        let domain = MergePolicy::try_from(api_policy.clone()).expect("valid policy");
        assert_eq!(domain, valid_policy());

        let back: api_repos::MergePolicy = domain.into();
        assert_eq!(back, api_policy);
    }

    #[test]
    fn try_from_validates_and_succeeds_for_valid_policy() {
        let api_policy = api_policy_matching_valid_policy();
        let domain = MergePolicy::try_from(api_policy).expect("valid policy");
        assert_eq!(domain, valid_policy());
    }

    #[test]
    fn try_from_rejects_invalid_policy() {
        // Same shape but count = 3 > any_of.len() = 2 on the second group.
        let mut bad = api_policy_matching_valid_policy();
        bad.reviewers[1].count = 3;
        let err = MergePolicy::try_from(bad).unwrap_err();
        assert!(
            matches!(
                err,
                MergePolicyValidationError::ReviewerCountExceedsAnyOf {
                    count: 3,
                    available: 2,
                    ..
                }
            ),
            "expected ReviewerCountExceedsAnyOf, got {err:?}",
        );
    }

    #[test]
    fn duplicate_labels_are_rejected() {
        let mut policy = valid_policy();
        policy.reviewers[1].label = Some("code-review".to_string());
        let err = validate_merge_policy(&policy).unwrap_err();
        assert!(
            matches!(
                err,
                MergePolicyValidationError::DuplicateReviewerLabel { ref label }
                    if label == "code-review"
            ),
            "expected DuplicateReviewerLabel, got {err:?}",
        );
    }

    #[test]
    fn empty_reviewer_any_of_is_rejected() {
        let policy = MergePolicy {
            reviewers: vec![ReviewerGroup {
                label: None,
                any_of: vec![],
                count: 1,
                exclude_author: true,
            }],
            mergers: None,
        };
        let err = validate_merge_policy(&policy).unwrap_err();
        assert!(
            matches!(
                err,
                MergePolicyValidationError::EmptyReviewerAnyOf { group_index: 0, .. }
            ),
            "expected EmptyReviewerAnyOf, got {err:?}",
        );
    }

    #[test]
    fn empty_merger_any_of_is_rejected() {
        let policy = MergePolicy {
            reviewers: vec![],
            mergers: Some(MergerRule { any_of: vec![] }),
        };
        let err = validate_merge_policy(&policy).unwrap_err();
        assert!(
            matches!(err, MergePolicyValidationError::EmptyMergerAnyOf),
            "expected EmptyMergerAnyOf, got {err:?}",
        );
    }

    #[test]
    fn zero_reviewer_count_is_rejected() {
        let policy = MergePolicy {
            reviewers: vec![ReviewerGroup {
                label: None,
                any_of: vec![user("alice")],
                count: 0,
                exclude_author: true,
            }],
            mergers: None,
        };
        let err = validate_merge_policy(&policy).unwrap_err();
        assert!(
            matches!(
                err,
                MergePolicyValidationError::ZeroReviewerCount { group_index: 0, .. }
            ),
            "expected ZeroReviewerCount, got {err:?}",
        );
    }

    #[test]
    fn count_exceeding_any_of_is_rejected() {
        let policy = MergePolicy {
            reviewers: vec![ReviewerGroup {
                label: None,
                any_of: vec![user("alice"), user("bob")],
                count: 3,
                exclude_author: true,
            }],
            mergers: None,
        };
        let err = validate_merge_policy(&policy).unwrap_err();
        assert!(
            matches!(
                err,
                MergePolicyValidationError::ReviewerCountExceedsAnyOf {
                    group_index: 0,
                    count: 3,
                    available: 2,
                    ..
                }
            ),
            "expected ReviewerCountExceedsAnyOf, got {err:?}",
        );
    }

    #[test]
    fn whitespace_username_is_rejected() {
        let policy = MergePolicy {
            reviewers: vec![ReviewerGroup {
                label: None,
                any_of: vec![user("alice smith")],
                count: 1,
                exclude_author: true,
            }],
            mergers: None,
        };
        let err = validate_merge_policy(&policy).unwrap_err();
        assert!(
            matches!(
                err,
                MergePolicyValidationError::InvalidUsername { ref name } if name == "alice smith"
            ),
            "expected InvalidUsername, got {err:?}",
        );
    }

    #[test]
    fn empty_username_is_rejected() {
        let policy = MergePolicy {
            reviewers: vec![ReviewerGroup {
                label: None,
                any_of: vec![user("")],
                count: 1,
                exclude_author: true,
            }],
            mergers: None,
        };
        let err = validate_merge_policy(&policy).unwrap_err();
        assert!(
            matches!(err, MergePolicyValidationError::InvalidUsername { ref name } if name.is_empty()),
            "expected InvalidUsername, got {err:?}",
        );
    }

    #[test]
    fn duplicate_principal_in_reviewer_group_is_rejected() {
        let policy = MergePolicy {
            reviewers: vec![ReviewerGroup {
                label: Some("dupes".to_string()),
                any_of: vec![user("alice"), user("alice")],
                count: 1,
                exclude_author: true,
            }],
            mergers: None,
        };
        let err = validate_merge_policy(&policy).unwrap_err();
        assert!(
            matches!(
                err,
                MergePolicyValidationError::DuplicatePrincipalInReviewerGroup {
                    group_index: 0,
                    ref principal,
                    ..
                } if principal == "alice"
            ),
            "expected DuplicatePrincipalInReviewerGroup, got {err:?}",
        );
    }

    #[test]
    fn duplicate_principal_in_mergers_is_rejected() {
        let policy = MergePolicy {
            reviewers: vec![],
            mergers: Some(MergerRule {
                any_of: vec![user("alice"), user("alice")],
            }),
        };
        let err = validate_merge_policy(&policy).unwrap_err();
        assert!(
            matches!(
                err,
                MergePolicyValidationError::DuplicatePrincipalInMergers { ref principal }
                    if principal == "alice"
            ),
            "expected DuplicatePrincipalInMergers, got {err:?}",
        );
    }

    #[test]
    fn valid_policy_passes_validation() {
        validate_merge_policy(&valid_policy()).expect("valid policy");
    }

    #[test]
    fn principal_serializes_as_bare_string_or_at_prefix() {
        let json_alice = serde_json::to_value(user("alice")).unwrap();
        assert_eq!(json_alice, serde_json::json!("alice"));

        let json_dynamic =
            serde_json::to_value(Principal::Dynamic(DynamicRef::PatchAuthor)).unwrap();
        assert_eq!(json_dynamic, serde_json::json!("@patch.author"));
    }
}
