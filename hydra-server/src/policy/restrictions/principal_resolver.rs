//! Resolves [`AssigneeRef`]s in a [`MergePolicy`] to concrete usernames at
//! merge-attempt time.
//!
//! `AssigneeRef::Static(Principal)` is the identity case (currently
//! resolved by surfacing the principal's display name).
//! `AssigneeRef::Dynamic(d)` is resolved against the current state of the
//! patch — never snapshotted. See `/designs/merge-time-constraints.md`
//! §4.4 for why resolution is live.
//!
//! Phase 5a of `/designs/actor-system-overhaul.md` widened the static
//! arm from `User(Username)` to the shared `Principal` (gaining `Agent`
//! and `External`). For now this resolver continues to surface only the
//! `name` / `username` field of the static principal; tightening
//! merger membership to match by kind (so an `Agent` config entry does
//! not silently match a `User` actor with the same string) is tracked
//! by Phase 6 of the design.

use hydra_common::Principal;
use hydra_common::api::v1::repositories::{AssigneeRef, DynamicRef};

use crate::domain::patches::Patch;
use crate::store::ReadOnlyStore;

/// Inputs available to [`resolve_principal`] / [`resolve_any_of`].
pub struct ResolutionContext<'a> {
    pub patch: &'a Patch,
    pub store: &'a dyn ReadOnlyStore,
}

/// A principal alongside the username it currently resolves to (if any).
///
/// `resolved_to` is `None` for `Dynamic` refs that cannot be resolved against
/// the current state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPrincipal {
    pub source: AssigneeRef,
    pub resolved_to: Option<String>,
}

/// Resolve a single [`AssigneeRef`] to a username string, or `None` if a
/// dynamic reference cannot be resolved.
pub fn resolve_principal(principal: &AssigneeRef, ctx: &ResolutionContext<'_>) -> Option<String> {
    match principal {
        AssigneeRef::Static(p) => Some(resolve_static_principal(p)),
        AssigneeRef::Dynamic(dref) => resolve_dynamic_ref(*dref, ctx),
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

fn resolve_static_principal(p: &Principal) -> String {
    match p {
        Principal::User { name } => name.as_str().to_string(),
        Principal::Agent { name } => name.as_str().to_string(),
        Principal::External { username, .. } => username.clone(),
    }
}

fn resolve_dynamic_ref(dref: DynamicRef, ctx: &ResolutionContext<'_>) -> Option<String> {
    match dref {
        DynamicRef::PatchAuthor => Some(ctx.patch.creator.as_str().to_string()),
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

    #[tokio::test]
    async fn resolves_user_principal_to_its_username() {
        let store = MemoryStore::new();
        let patch = make_patch("author");
        let c = ctx(&patch, &store);

        let p = user_ref("alice");
        assert_eq!(resolve_principal(&p, &c), Some("alice".to_string()));
    }

    #[tokio::test]
    async fn resolves_agent_principal_to_its_agent_name() {
        let store = MemoryStore::new();
        let patch = make_patch("author");
        let c = ctx(&patch, &store);

        let p = agent_ref("swe");
        assert_eq!(resolve_principal(&p, &c), Some("swe".to_string()));
    }

    #[tokio::test]
    async fn patch_author_dynamic_ref_resolves_to_patch_creator() {
        let store = MemoryStore::new();
        let patch = make_patch("author");
        let c = ctx(&patch, &store);

        let p = AssigneeRef::Dynamic(DynamicRef::PatchAuthor);
        assert_eq!(resolve_principal(&p, &c), Some("author".to_string()));
    }

    #[tokio::test]
    async fn resolve_any_of_preserves_order_and_source() {
        let store = MemoryStore::new();
        let patch = make_patch("author");
        let c = ctx(&patch, &store);

        let principals = vec![
            user_ref("alice"),
            AssigneeRef::Dynamic(DynamicRef::PatchAuthor),
        ];

        let resolved = resolve_any_of(&principals, &c);
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].source, principals[0]);
        assert_eq!(resolved[0].resolved_to, Some("alice".to_string()));
        assert_eq!(resolved[1].source, principals[1]);
        assert_eq!(resolved[1].resolved_to, Some("author".to_string()));
    }
}
