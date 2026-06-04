//! Resolves merge-policy principal references like `@patch.author` /
//! `@parent_issue.assignee` against the live state of the patch/issue at
//! evaluation time.
//!
//! `AssigneeRef::Static(Principal)` is the identity case — the configured
//! principal is the resolution. `AssigneeRef::Dynamic(d)` is resolved against
//! the current state of the patch — never snapshotted. Snapshotted resolution
//! would let stale references stand if the patch/issue mutates between policy
//! evaluation and merge (e.g. assignee changes after the policy is read),
//! silently approving the wrong actor.
//!
//! The resolved value carries the full typed [`Principal`] so downstream
//! matching (`mergers.any_of`, reviewer-group quorum) is kind-aware —
//! an `Agent` config entry never silently matches a `User` actor with
//! the same string.

use hydra_common::Principal;
use hydra_common::api::v1::repositories::{AssigneeRef, DynamicRef};
use hydra_common::api::v1::users::Username;

use crate::domain::patches::Patch;
use crate::store::ReadOnlyStore;

/// Inputs available to [`resolve_principal`] / [`resolve_any_of`].
pub struct ResolutionContext<'a> {
    pub patch: &'a Patch,
    pub store: &'a dyn ReadOnlyStore,
}

/// A principal alongside the [`Principal`] it currently resolves to (if any).
///
/// `resolved_to` is `None` for `Dynamic` refs that cannot be resolved against
/// the current state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPrincipal {
    pub source: AssigneeRef,
    pub resolved_to: Option<Principal>,
}

/// Resolve a single [`AssigneeRef`] to a typed [`Principal`], or `None` if a
/// dynamic reference cannot be resolved against the current patch state.
pub fn resolve_principal(
    principal: &AssigneeRef,
    ctx: &ResolutionContext<'_>,
) -> Option<Principal> {
    match principal {
        AssigneeRef::Static(p) => Some(resolve_static_principal(p)),
        AssigneeRef::Dynamic(dref) => ctx.resolve_dynamic_ref(*dref),
    }
}

/// Resolve every principal in `any_of`, preserving order and the original
/// [`AssigneeRef`] alongside each result.
pub fn resolve_any_of(
    principals: &[AssigneeRef],
    ctx: &ResolutionContext<'_>,
) -> Vec<ResolvedPrincipal> {
    principals
        .iter()
        .map(|p| ResolvedPrincipal {
            source: p.clone(),
            resolved_to: resolve_principal(p, ctx),
        })
        .collect()
}

fn resolve_static_principal(p: &Principal) -> Principal {
    p.clone()
}

impl ResolutionContext<'_> {
    fn resolve_dynamic_ref(&self, dref: DynamicRef) -> Option<Principal> {
        match dref {
            // `Patch.creator: Username` so the patch-creator dynamic ref always
            // resolves to `Principal::User` — patches today are not created
            // by `Agent` or `External` identities.
            DynamicRef::PatchCreator => Some(Principal::User {
                name: Username::from(self.patch.creator.as_str()),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::patches::{Patch, PatchStatus};
    use crate::domain::users::Username;
    use crate::store::MemoryStore;
    use hydra_common::RepoName;
    use hydra_common::api::v1::agents::AgentName;
    use hydra_common::api::v1::users::Username as ApiUsername;
    use std::str::FromStr;

    fn make_patch(creator: &str) -> Patch {
        Patch {
            title: "t".to_string(),
            description: String::new(),
            diff: String::new(),
            status: PatchStatus::Open,
            is_automatic_backup: false,
            branch_name: None,
            service_repo_name: RepoName::from_str("test/repo").unwrap(),
            creator: Username::from(creator),
            github: None,
            commit_range: None,
            base_branch: None,
            reviews: Vec::new(),
            deleted: false,
        }
    }

    fn ctx<'a>(patch: &'a Patch, store: &'a dyn ReadOnlyStore) -> ResolutionContext<'a> {
        ResolutionContext { patch, store }
    }

    fn user_ref(name: &str) -> AssigneeRef {
        AssigneeRef::Static(Principal::User {
            name: ApiUsername::try_new(name).unwrap(),
        })
    }

    fn agent_ref(name: &str) -> AssigneeRef {
        AssigneeRef::Static(Principal::Agent {
            name: AgentName::try_new(name).unwrap(),
        })
    }

    fn user_principal(name: &str) -> Principal {
        Principal::User {
            name: ApiUsername::try_new(name).unwrap(),
        }
    }

    fn agent_principal(name: &str) -> Principal {
        Principal::Agent {
            name: AgentName::try_new(name).unwrap(),
        }
    }

    #[tokio::test]
    async fn resolves_user_principal_to_typed_user() {
        let store = MemoryStore::new();
        let patch = make_patch("author");
        let c = ctx(&patch, &store);

        let p = user_ref("alice");
        assert_eq!(resolve_principal(&p, &c), Some(user_principal("alice")));
    }

    #[tokio::test]
    async fn resolves_agent_principal_to_typed_agent() {
        let store = MemoryStore::new();
        let patch = make_patch("author");
        let c = ctx(&patch, &store);

        let p = agent_ref("swe");
        assert_eq!(resolve_principal(&p, &c), Some(agent_principal("swe")));
    }

    #[tokio::test]
    async fn patch_creator_dynamic_ref_resolves_to_user_principal_for_creator() {
        let store = MemoryStore::new();
        let patch = make_patch("author");
        let c = ctx(&patch, &store);

        let p = AssigneeRef::Dynamic(DynamicRef::PatchCreator);
        assert_eq!(resolve_principal(&p, &c), Some(user_principal("author")));
    }

    #[tokio::test]
    async fn resolve_any_of_preserves_order_and_source() {
        let store = MemoryStore::new();
        let patch = make_patch("author");
        let c = ctx(&patch, &store);

        let principals = vec![
            user_ref("alice"),
            AssigneeRef::Dynamic(DynamicRef::PatchCreator),
        ];

        let resolved = resolve_any_of(&principals, &c);
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].source, principals[0]);
        assert_eq!(resolved[0].resolved_to, Some(user_principal("alice")));
        assert_eq!(resolved[1].source, principals[1]);
        assert_eq!(resolved[1].resolved_to, Some(user_principal("author")));
    }

    #[tokio::test]
    async fn patch_creator_alias_round_trip_resolves_identically() {
        // Verifies the lenient `@patch.author` alias resolves to the same
        // typed principal as the canonical `@patch.creator` once parsed.
        let store = MemoryStore::new();
        let patch = make_patch("author");
        let c = ctx(&patch, &store);

        let from_alias: AssigneeRef = serde_json::from_str("\"@patch.author\"").unwrap();
        let from_canonical: AssigneeRef = serde_json::from_str("\"@patch.creator\"").unwrap();
        assert_eq!(
            resolve_principal(&from_alias, &c),
            resolve_principal(&from_canonical, &c),
        );
    }
}
