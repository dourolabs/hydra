use hydra_common::Principal;
use hydra_common::api::v1 as api;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use thiserror::Error;

/// Domain mirror of [`api::repositories::DynamicRef`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DynamicRef {
    PatchCreator,
}

impl DynamicRef {
    pub fn shorthand(self) -> &'static str {
        match self {
            DynamicRef::PatchCreator => "patch.creator",
        }
    }
}

impl fmt::Display for DynamicRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@{}", self.shorthand())
    }
}

/// Domain mirror of [`api::repositories::AssigneeRef`] — a static
/// [`Principal`] or a dynamic ref resolved at merge-attempt time.
///
/// The domain layer is constructed via the validating
/// [`MergePolicy::try_from`] API → domain conversion; there is no
/// stand-alone serde wire format because the domain `MergePolicy` is
/// stored as the API-layer type in the repositories table.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "ref_kind", rename_all = "snake_case")]
pub enum AssigneeRef {
    Static(Principal),
    Dynamic(DynamicRef),
}

impl fmt::Display for AssigneeRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssigneeRef::Static(p) => p.fmt(f),
            AssigneeRef::Dynamic(d) => d.fmt(f),
        }
    }
}

/// Domain mirror of [`api::repositories::ReviewerGroup`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewerGroup {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub any_of: Vec<AssigneeRef>,
    pub count: u32,
    pub exclude_author: bool,
}

/// Domain mirror of [`api::repositories::MergerRule`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergerRule {
    pub any_of: Vec<AssigneeRef>,
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

/// Validate a [`MergePolicy`] for structural and semantic correctness.
///
/// Returns the first failure encountered; callers are expected to surface the
/// error to the user rather than continue with a partially valid policy.
///
/// Per-principal *existence* validation (i.e. that the named user / agent
/// resolves to an actual store row) is done separately in the app layer via
/// `Store::principal_exists`; this function only checks intra-policy shape
/// invariants.
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

        let mut seen: HashSet<&AssigneeRef> = HashSet::new();
        for principal in &group.any_of {
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
        let mut seen: HashSet<&AssigneeRef> = HashSet::new();
        for principal in &mergers.any_of {
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
            api::repositories::DynamicRef::PatchCreator => DynamicRef::PatchCreator,
        }
    }
}

impl From<DynamicRef> for api::repositories::DynamicRef {
    fn from(value: DynamicRef) -> Self {
        match value {
            DynamicRef::PatchCreator => api::repositories::DynamicRef::PatchCreator,
        }
    }
}

impl From<api::repositories::AssigneeRef> for AssigneeRef {
    fn from(value: api::repositories::AssigneeRef) -> Self {
        match value {
            api::repositories::AssigneeRef::Static(p) => AssigneeRef::Static(p),
            api::repositories::AssigneeRef::Dynamic(d) => AssigneeRef::Dynamic(d.into()),
        }
    }
}

impl From<AssigneeRef> for api::repositories::AssigneeRef {
    fn from(value: AssigneeRef) -> Self {
        match value {
            AssigneeRef::Static(p) => api::repositories::AssigneeRef::Static(p),
            AssigneeRef::Dynamic(d) => api::repositories::AssigneeRef::Dynamic(d.into()),
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
    use hydra_common::api::v1::agents::AgentName;
    use hydra_common::api::v1::repositories as api_repos;
    use hydra_common::api::v1::users::Username;

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

    fn api_user(name: &str) -> api_repos::AssigneeRef {
        api_repos::AssigneeRef::Static(Principal::User {
            name: Username::try_new(name).unwrap(),
        })
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
                any_of: vec![
                    AssigneeRef::Dynamic(DynamicRef::PatchCreator),
                    user("alice"),
                ],
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
                    api_repos::AssigneeRef::Dynamic(api_repos::DynamicRef::PatchCreator),
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
                } if principal == "users/alice"
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
                    if principal == "users/alice"
            ),
            "expected DuplicatePrincipalInMergers, got {err:?}",
        );
    }

    #[test]
    fn valid_policy_passes_validation() {
        validate_merge_policy(&valid_policy()).expect("valid policy");
    }

    #[test]
    fn agent_principal_validates_intra_policy_invariants() {
        // The validation function does not enforce that agents are
        // distinct from users — the type system already separates them
        // and `principal_exists` handles cross-table lookups. Confirm a
        // policy mixing the two passes intra-shape validation.
        let policy = MergePolicy {
            reviewers: vec![ReviewerGroup {
                label: None,
                any_of: vec![user("alice"), agent("swe")],
                count: 1,
                exclude_author: false,
            }],
            mergers: None,
        };
        validate_merge_policy(&policy).expect("mixed user/agent policy must validate");
    }
}
