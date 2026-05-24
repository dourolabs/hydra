//! Resolves [`Principal`]s in a [`MergePolicy`] to concrete usernames at
//! merge-attempt time.
//!
//! `Principal::User(u)` is the identity case. `Principal::Dynamic(d)` is
//! resolved against the current state of the patch — never snapshotted. See
//! `/designs/merge-time-constraints.md` §4.4 for why resolution is live.

use hydra_common::api::v1::repositories::{DynamicRef, Principal};

use crate::domain::patches::Patch;
use crate::store::ReadOnlyStore;

/// Inputs available to [`resolve_principal`] / [`resolve_any_of`].
pub struct ResolutionContext<'a> {
    pub patch: &'a Patch,
    pub patch_id: &'a hydra_common::PatchId,
    pub store: &'a dyn ReadOnlyStore,
}

/// A principal alongside the username it currently resolves to (if any).
///
/// `resolved_to` is `None` for `Dynamic` refs that cannot be resolved against
/// the current state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPrincipal {
    pub source: Principal,
    pub resolved_to: Option<String>,
}

/// Resolve a single [`Principal`] to a username string, or `None` if a
/// dynamic reference cannot be resolved.
pub fn resolve_principal(principal: &Principal, ctx: &ResolutionContext<'_>) -> Option<String> {
    match principal {
        Principal::User(username) => Some(username.as_str().to_string()),
        Principal::Dynamic(dref) => resolve_dynamic_ref(*dref, ctx),
    }
}

/// Resolve every principal in `any_of`, preserving order and the original
/// [`Principal`] alongside each result.
pub fn resolve_any_of(
    principals: &[Principal],
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
            created_by: None,
            creator: Username::from(creator),
            github: None,
            commit_range: None,
            base_branch: None,
            reviews: Vec::new(),
            deleted: false,
        }
    }

    fn ctx<'a>(
        patch: &'a Patch,
        patch_id: &'a hydra_common::PatchId,
        store: &'a dyn ReadOnlyStore,
    ) -> ResolutionContext<'a> {
        ResolutionContext {
            patch,
            patch_id,
            store,
        }
    }

    #[tokio::test]
    async fn resolves_user_principal_to_its_username() {
        let store = MemoryStore::new();
        let patch = make_patch("author");
        let patch_id = hydra_common::PatchId::new();
        let c = ctx(&patch, &patch_id, &store);

        let p = Principal::User(hydra_common::api::v1::users::Username::from("alice"));
        assert_eq!(resolve_principal(&p, &c), Some("alice".to_string()));
    }

    #[tokio::test]
    async fn patch_author_dynamic_ref_resolves_to_patch_creator() {
        let store = MemoryStore::new();
        let patch = make_patch("author");
        let patch_id = hydra_common::PatchId::new();
        let c = ctx(&patch, &patch_id, &store);

        let p = Principal::Dynamic(DynamicRef::PatchAuthor);
        assert_eq!(resolve_principal(&p, &c), Some("author".to_string()));
    }

    #[tokio::test]
    async fn resolve_any_of_preserves_order_and_source() {
        let store = MemoryStore::new();
        let patch = make_patch("author");
        let patch_id = hydra_common::PatchId::new();
        let c = ctx(&patch, &patch_id, &store);

        let principals = vec![
            Principal::User(hydra_common::api::v1::users::Username::from("alice")),
            Principal::Dynamic(DynamicRef::PatchAuthor),
        ];

        let resolved = resolve_any_of(&principals, &c);
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].source, principals[0]);
        assert_eq!(resolved[0].resolved_to, Some("alice".to_string()));
        assert_eq!(resolved[1].source, principals[1]);
        assert_eq!(resolved[1].resolved_to, Some("author".to_string()));
    }
}
