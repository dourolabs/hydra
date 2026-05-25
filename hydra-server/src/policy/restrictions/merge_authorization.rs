//! Server-authoritative gate on patch status transitions to `Merged`.
//!
//! See `/designs/merge-time-constraints.md` §4.3 for the high-level flow and
//! §4.5 for the structured error shape carried in
//! [`PolicyViolation::message`].
//!
//! The restriction is a strict no-op for repositories without a configured
//! `merge_policy`, preserving backward-compatibility with every repo today.

use async_trait::async_trait;
use hydra_common::ActorId;
use hydra_common::api::v1::merge_check::{
    BlockedAtLayer, EligiblePrincipal, MergeBlockedCode, MergeBlockedError, MergeBlockedReason,
    SuggestedAction,
};
use hydra_common::api::v1::repositories::{MergePolicy, Principal, ReviewerGroup};
use hydra_common::review_utils::is_review_non_stale;

use crate::domain::actors::ActorRef;
use crate::domain::patches::{Patch, PatchStatus};
use crate::policy::context::{Operation, OperationPayload, RestrictionContext};
use crate::policy::restrictions::principal_resolver::{
    ResolutionContext, ResolvedPrincipal, resolve_any_of,
};
use crate::policy::{PolicyViolation, Restriction};
use crate::store::ReadOnlyStore;

const RESTRICTION_NAME: &str = "merge_authorization";

/// Layer priority used to gate the response: the response carries failures
/// from the first layer in this list that has any. Future layers slot in by
/// extending this constant (see design §4.5).
const LAYER_PRIORITY: &[&str] = &["reviews", "mergers"];

/// Restriction that enforces the repository's [`MergePolicy`] on every
/// transition INTO `PatchStatus::Merged`.
#[derive(Default)]
pub struct MergeAuthorizationRestriction;

impl MergeAuthorizationRestriction {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Restriction for MergeAuthorizationRestriction {
    fn name(&self) -> &str {
        RESTRICTION_NAME
    }

    async fn evaluate(&self, ctx: &RestrictionContext<'_>) -> Result<(), PolicyViolation> {
        // 1. Status-transition gating: only fires on UpdatePatch transitions
        //    INTO Merged. Create-as-Merged would arguably qualify but is not
        //    a path the API exposes; restrict to UpdatePatch here to match
        //    the design (§4.3).
        if ctx.operation != Operation::UpdatePatch {
            return Ok(());
        }
        let OperationPayload::Patch { patch_id, new, old } = ctx.payload else {
            return Ok(());
        };
        if new.status != PatchStatus::Merged {
            return Ok(());
        }
        if let Some(old_patch) = old {
            if old_patch.status == PatchStatus::Merged {
                return Ok(());
            }
        }
        let Some(patch_id) = patch_id else {
            // UpdatePatch always carries an id; defensive bail.
            return Ok(());
        };

        // 2. Load the repository; if no merge_policy is configured, allow.
        let repository = ctx
            .store
            .get_repository(&new.service_repo_name, false)
            .await
            .map_err(|e| internal_violation(format!("failed to load repository: {e}")))?;
        let Some(policy) = repository.item.merge_policy.as_ref() else {
            return Ok(());
        };

        // 3. Build the resolution context for principal resolution.
        let resolution_ctx = ResolutionContext {
            patch: new,
            store: ctx.store,
        };

        // 4. Layer 1 — reviews. Collect failures across every group.
        let review_failures = evaluate_reviews_layer(policy, &resolution_ctx, patch_id);
        if !review_failures.is_empty() {
            debug_assert_eq!(LAYER_PRIORITY[0], "reviews");
            return Err(build_violation(
                patch_id,
                BlockedAtLayer::Reviews,
                review_failures,
            ));
        }

        // 5. Layer 2 — mergers. Only evaluated when every reviewer group
        //    is satisfied.
        if let Some(failure) =
            evaluate_mergers_layer(policy, &resolution_ctx, ctx.actor, patch_id).await
        {
            debug_assert_eq!(LAYER_PRIORITY[1], "mergers");
            return Err(build_violation(
                patch_id,
                BlockedAtLayer::Mergers,
                vec![failure],
            ));
        }

        Ok(())
    }
}

/// Build one `MissingApprovals` reason per unsatisfied reviewer group.
fn evaluate_reviews_layer(
    policy: &MergePolicy,
    ctx: &ResolutionContext<'_>,
    patch_id: &hydra_common::PatchId,
) -> Vec<MergeBlockedReason> {
    let mut failures = Vec::new();
    for (group_index, group) in policy.reviewers.iter().enumerate() {
        if let Some(reason) = evaluate_reviewer_group(group, group_index, ctx, patch_id) {
            failures.push(reason);
        }
    }
    failures
}

/// Returns `Some(MissingApprovals)` if the group is unsatisfied, else `None`.
fn evaluate_reviewer_group(
    group: &ReviewerGroup,
    group_index: usize,
    ctx: &ResolutionContext<'_>,
    patch_id: &hydra_common::PatchId,
) -> Option<MergeBlockedReason> {
    let resolved = resolve_any_of(&group.any_of, ctx);
    let author = ctx.patch.creator.as_str();

    // Apply author-exclusion to the eligible set used for both counting and
    // for the error's `suggested_action.assign_to_one_of` list.
    let eligible: Vec<&ResolvedPrincipal> = resolved
        .iter()
        .filter(|rp| {
            if !group.exclude_author {
                return true;
            }
            match rp.resolved_to.as_deref() {
                Some(name) => !name.eq_ignore_ascii_case(author),
                None => true,
            }
        })
        .collect();

    // Collect approving non-stale reviews per *eligible principal* (so a
    // single author cannot satisfy two slots).
    let approving_names = approving_non_stale_authors(ctx.patch);
    let mut satisfied_names: Vec<String> = Vec::new();
    for rp in &eligible {
        if let Some(name) = rp.resolved_to.as_deref() {
            if approving_names.iter().any(|a| a.eq_ignore_ascii_case(name))
                && !satisfied_names.iter().any(|s| s.eq_ignore_ascii_case(name))
            {
                satisfied_names.push(name.to_string());
            }
        }
    }

    if (satisfied_names.len() as u32) >= group.count {
        return None;
    }

    // Build the error reason describing the unsatisfied group.
    let eligible_principals: Vec<EligiblePrincipal> = eligible
        .iter()
        .map(|rp| to_eligible_principal(rp))
        .collect();
    let needed = group.count;
    let title_hint = match &group.label {
        Some(label) => format!("Review {patch_id} ({label})"),
        None => format!("Review {patch_id}"),
    };
    let current_approvals: Vec<String> = satisfied_names
        .iter()
        .take(group.count as usize)
        .cloned()
        .collect();

    // `assign_to_one_of` lists eligible principals not already counted as
    // approving (so the SWE doesn't re-assign an existing reviewer).
    let assign_to_one_of: Vec<String> = eligible
        .iter()
        .filter_map(|rp| rp.resolved_to.clone())
        .filter(|name| !satisfied_names.iter().any(|s| s.eq_ignore_ascii_case(name)))
        .collect();

    Some(MergeBlockedReason::MissingApprovals {
        group_index: group_index as u32,
        label: group.label.clone(),
        eligible_principals,
        current_approvals,
        needed,
        suggested_action: SuggestedAction::FileReviewRequest {
            assign_to_one_of,
            title_hint,
        },
    })
}

/// Returns `Some(NotInMergers)` if the actor isn't in the resolved mergers
/// list, `None` otherwise (including when no `mergers` rule is configured).
async fn evaluate_mergers_layer(
    policy: &MergePolicy,
    ctx: &ResolutionContext<'_>,
    actor: &ActorRef,
    _patch_id: &hydra_common::PatchId,
) -> Option<MergeBlockedReason> {
    let mergers = policy.mergers.as_ref()?;
    let resolved = resolve_any_of(&mergers.any_of, ctx);
    let actor_username = actor_username(actor, ctx.store).await;

    if let Some(actor_name) = actor_username.as_deref() {
        let matches_any = resolved.iter().any(|rp| {
            rp.resolved_to
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case(actor_name))
        });
        if matches_any {
            return None;
        }
    }

    let allowed_mergers: Vec<EligiblePrincipal> =
        resolved.iter().map(to_eligible_principal).collect();
    let assign_to_one_of: Vec<String> = resolved
        .iter()
        .filter_map(|rp| rp.resolved_to.clone())
        .collect();

    Some(MergeBlockedReason::NotInMergers {
        actor: actor.display_name(),
        allowed_mergers,
        suggested_action: SuggestedAction::FileMergeRequest { assign_to_one_of },
    })
}

fn to_eligible_principal(rp: &ResolvedPrincipal) -> EligiblePrincipal {
    match &rp.source {
        Principal::User(u) => EligiblePrincipal::User {
            username: u.as_str().to_string(),
        },
        Principal::Dynamic(dref) => EligiblePrincipal::Dynamic {
            reference: *dref,
            resolved_to: rp.resolved_to.clone(),
        },
    }
}

/// Collect usernames whose latest non-stale review on the patch is an
/// approval. Matches the semantics of `has_approved_non_dismissed_review`
/// (case-insensitive on author) but returns the set rather than a boolean
/// so reviewer-group quorum counting can deduplicate.
fn approving_non_stale_authors(patch: &Patch) -> Vec<String> {
    let api_reviews: Vec<hydra_common::api::v1::patches::Review> =
        patch.reviews.iter().cloned().map(Into::into).collect();

    // Patch version history is not available to the restriction (the
    // ReadOnlyStore is by-design hidden from `is_review_non_stale`'s
    // signature in this code path), so we evaluate staleness with an
    // empty version history. The shared predicate treats an empty
    // history as "no commit-range changes have occurred", which matches
    // today's CLI behaviour at merge time.
    let versions: Vec<hydra_common::api::v1::patches::PatchVersionRecord> = Vec::new();

    let mut authors: Vec<String> = Vec::new();
    for review in &api_reviews {
        let key = review.author.to_ascii_lowercase();
        if authors.iter().any(|a| a.eq_ignore_ascii_case(&key)) {
            continue;
        }
        if let Some(latest) = hydra_common::review_utils::find_latest_review_by_author(
            &api_reviews,
            &review.author,
            None,
        ) {
            if latest.is_approved && is_review_non_stale(latest, &versions) {
                authors.push(latest.author.clone());
            }
        }
    }
    authors
}

/// Best-effort mapping from `ActorRef` to the username we should match
/// against `mergers.any_of`. Returns `None` when no username can be
/// derived — the caller treats that as "not in mergers".
async fn actor_username(actor: &ActorRef, store: &dyn ReadOnlyStore) -> Option<String> {
    let actor_id = actor.on_behalf_of()?;
    match actor_id {
        ActorId::Username(u) => Some(u.as_str().to_string()),
        // Phase-2 `Adhoc(sid)` matches the legacy `Session(sid)` arm:
        // both are sessions without a registered agent identity, so
        // the merger username is the session's creator.
        ActorId::Session(sid) | ActorId::Adhoc(sid) => store
            .get_session(&sid, false)
            .await
            .ok()
            .map(|s| s.item.creator.as_str().to_string()),
        ActorId::Issue(iid) => store
            .get_issue(&iid, false)
            .await
            .ok()
            .map(|i| i.item.creator.as_str().to_string()),
        // Agent-spawned sessions share an actor row keyed by agent
        // name; resolve via that row's creator so an agent acting on
        // behalf of `alice` is matched as `alice` for merger
        // membership. This is the closest Phase-2 approximation —
        // Phase 6 of `/designs/actor-system-overhaul.md` revisits
        // merger semantics with the typed `Principal`.
        ActorId::Agent(_) => store
            .get_actor(&actor_id.to_string())
            .await
            .ok()
            .map(|a| a.item.creator.as_str().to_string()),
        ActorId::Service(_) => None,
        // `User` / `External` / `Legacy` aren't produced by any
        // hydra-server call site that flows through merge auth yet.
        // Phase 6 updates this resolver.
        ActorId::User(_) | ActorId::External { .. } | ActorId::Legacy(_) => None,
    }
}

fn build_violation(
    patch_id: &hydra_common::PatchId,
    blocked_at_layer: BlockedAtLayer,
    reasons: Vec<MergeBlockedReason>,
) -> PolicyViolation {
    let body = MergeBlockedError {
        code: MergeBlockedCode::MergeBlocked,
        patch_id: patch_id.clone(),
        blocked_at_layer,
        reasons,
    };
    let message = serde_json::to_string(&body)
        .unwrap_or_else(|e| format!("merge_blocked: failed to serialize error payload: {e}"));
    PolicyViolation {
        policy_name: RESTRICTION_NAME.to_string(),
        message,
    }
}

fn internal_violation(message: String) -> PolicyViolation {
    PolicyViolation {
        policy_name: RESTRICTION_NAME.to_string(),
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::actors::ActorRef as DomainActorRef;
    use crate::domain::patches::{Patch, PatchStatus, Review};
    use crate::domain::users::Username;
    use crate::policy::context::{Operation, OperationPayload, RestrictionContext};
    use crate::store::{MemoryStore, Store};
    use chrono::Utc;
    use hydra_common::api::v1::merge_check::{
        BlockedAtLayer, MergeBlockedError, MergeBlockedReason,
    };
    use hydra_common::api::v1::repositories::{
        MergePolicy, MergerRule, Principal as ApiPrincipal, ReviewerGroup,
    };
    use hydra_common::api::v1::users::Username as ApiUsername;
    use hydra_common::{ActorId, ActorRef as CommonActorRef, Repository};
    use std::str::FromStr;

    fn repo_name() -> hydra_common::RepoName {
        hydra_common::RepoName::from_str("test/repo").unwrap()
    }

    fn make_patch_with(reviews: Vec<Review>, creator: &str) -> Patch {
        Patch {
            title: "t".to_string(),
            description: String::new(),
            diff: String::new(),
            status: PatchStatus::Merged,
            is_automatic_backup: false,
            branch_name: None,
            service_repo_name: repo_name(),
            creator: Username::from(creator),
            github: None,
            commit_range: None,
            base_branch: None,
            reviews,
            deleted: false,
        }
    }

    fn old_open(creator: &str) -> Patch {
        let mut p = make_patch_with(Vec::new(), creator);
        p.status = PatchStatus::Open;
        p
    }

    fn approval(author: &str) -> Review {
        Review::new(
            "LGTM".to_string(),
            true,
            author.to_string(),
            Some(Utc::now()),
        )
    }

    async fn add_repo_with_policy(
        store: &MemoryStore,
        policy: Option<MergePolicy>,
    ) -> hydra_common::RepoName {
        let name = repo_name();
        let mut repo = Repository::new("https://example/repo.git".to_string(), None, None);
        repo.merge_policy = policy;
        store
            .add_repository(name.clone(), repo, &DomainActorRef::test())
            .await
            .expect("add repository");
        name
    }

    fn user(name: &str) -> ApiPrincipal {
        ApiPrincipal::User(ApiUsername::from(name))
    }

    fn user_actor(name: &str) -> CommonActorRef {
        CommonActorRef::Authenticated {
            actor_id: ActorId::Username(ApiUsername::from(name)),
            session_id: None,
        }
    }

    async fn evaluate(
        restriction: &MergeAuthorizationRestriction,
        store: &MemoryStore,
        patch: Patch,
        old: Option<Patch>,
        actor: &DomainActorRef,
    ) -> Result<(), PolicyViolation> {
        let payload = OperationPayload::Patch {
            patch_id: Some(hydra_common::PatchId::new()),
            new: patch,
            old,
        };
        let ctx = RestrictionContext {
            operation: Operation::UpdatePatch,
            actor,
            payload: &payload,
            store,
        };
        restriction.evaluate(&ctx).await
    }

    fn parse_message(violation: &PolicyViolation) -> MergeBlockedError {
        serde_json::from_str::<MergeBlockedError>(&violation.message)
            .expect("PolicyViolation.message must be a JSON-serialised MergeBlockedError")
    }

    // ---- §8.1: no-policy bypass -----------------------------------------

    #[tokio::test]
    async fn no_policy_allows_merge() {
        let store = MemoryStore::new();
        add_repo_with_policy(&store, None).await;
        let r = MergeAuthorizationRestriction::new();
        let patch = make_patch_with(Vec::new(), "author");
        let old = old_open("author");
        assert!(
            evaluate(&r, &store, patch, Some(old), &user_actor("anyone"))
                .await
                .is_ok()
        );
    }

    // ---- §8.2: single static-user reviewer required ---------------------

    #[tokio::test]
    async fn single_reviewer_blocks_without_approval() {
        let store = MemoryStore::new();
        let policy = MergePolicy {
            reviewers: vec![ReviewerGroup {
                label: Some("code-review".to_string()),
                any_of: vec![user("reviewer")],
                count: 1,
                exclude_author: true,
            }],
            mergers: None,
        };
        add_repo_with_policy(&store, Some(policy)).await;

        let r = MergeAuthorizationRestriction::new();
        let patch = make_patch_with(Vec::new(), "author");
        let old = old_open("author");
        let err = evaluate(&r, &store, patch, Some(old), &user_actor("author"))
            .await
            .unwrap_err();
        let body = parse_message(&err);
        assert_eq!(body.blocked_at_layer, BlockedAtLayer::Reviews);
        assert_eq!(body.reasons.len(), 1);
    }

    #[tokio::test]
    async fn single_reviewer_allows_with_approval() {
        let store = MemoryStore::new();
        let policy = MergePolicy {
            reviewers: vec![ReviewerGroup {
                label: None,
                any_of: vec![user("reviewer")],
                count: 1,
                exclude_author: true,
            }],
            mergers: None,
        };
        add_repo_with_policy(&store, Some(policy)).await;

        let r = MergeAuthorizationRestriction::new();
        let patch = make_patch_with(vec![approval("reviewer")], "author");
        let old = old_open("author");
        evaluate(&r, &store, patch, Some(old), &user_actor("anyone"))
            .await
            .expect("merge should be allowed");
    }

    // ---- §8.3: not-in-mergers ------------------------------------------

    #[tokio::test]
    async fn mergers_blocks_actor_not_in_list() {
        let store = MemoryStore::new();
        let policy = MergePolicy {
            reviewers: vec![],
            mergers: Some(MergerRule {
                any_of: vec![user("alice")],
            }),
        };
        add_repo_with_policy(&store, Some(policy)).await;

        let r = MergeAuthorizationRestriction::new();
        let patch = make_patch_with(Vec::new(), "author");
        let old = old_open("author");
        let err = evaluate(&r, &store, patch, Some(old), &user_actor("swe-session-x"))
            .await
            .unwrap_err();
        let body = parse_message(&err);
        assert_eq!(body.blocked_at_layer, BlockedAtLayer::Mergers);
        assert_eq!(body.reasons.len(), 1);
        match &body.reasons[0] {
            MergeBlockedReason::NotInMergers { actor, .. } => {
                assert_eq!(actor, "swe-session-x");
            }
            other => panic!("expected NotInMergers, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn mergers_allows_listed_actor() {
        let store = MemoryStore::new();
        let policy = MergePolicy {
            reviewers: vec![],
            mergers: Some(MergerRule {
                any_of: vec![user("alice")],
            }),
        };
        add_repo_with_policy(&store, Some(policy)).await;

        let r = MergeAuthorizationRestriction::new();
        let patch = make_patch_with(Vec::new(), "author");
        let old = old_open("author");
        evaluate(&r, &store, patch, Some(old), &user_actor("alice"))
            .await
            .expect("alice may merge");
    }

    // ---- §8.4: quorum --------------------------------------------------

    #[tokio::test]
    async fn quorum_one_approval_is_insufficient() {
        let store = MemoryStore::new();
        let policy = MergePolicy {
            reviewers: vec![ReviewerGroup {
                label: None,
                any_of: vec![user("a"), user("b"), user("c")],
                count: 2,
                exclude_author: true,
            }],
            mergers: None,
        };
        add_repo_with_policy(&store, Some(policy)).await;
        let r = MergeAuthorizationRestriction::new();

        let patch = make_patch_with(vec![approval("a")], "author");
        let old = old_open("author");
        assert!(
            evaluate(&r, &store, patch, Some(old), &user_actor("author"))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn quorum_two_distinct_approvals_suffice() {
        let store = MemoryStore::new();
        let policy = MergePolicy {
            reviewers: vec![ReviewerGroup {
                label: None,
                any_of: vec![user("a"), user("b"), user("c")],
                count: 2,
                exclude_author: true,
            }],
            mergers: None,
        };
        add_repo_with_policy(&store, Some(policy)).await;
        let r = MergeAuthorizationRestriction::new();

        let patch = make_patch_with(vec![approval("a"), approval("b")], "author");
        let old = old_open("author");
        evaluate(&r, &store, patch, Some(old), &user_actor("author"))
            .await
            .expect("two distinct approvals satisfy quorum");
    }

    // ---- §8.5: author exclusion ----------------------------------------

    #[tokio::test]
    async fn author_exclusion_drops_author_review() {
        let store = MemoryStore::new();
        let policy = MergePolicy {
            reviewers: vec![ReviewerGroup {
                label: None,
                any_of: vec![user("author"), user("alice")],
                count: 1,
                exclude_author: true,
            }],
            mergers: None,
        };
        add_repo_with_policy(&store, Some(policy)).await;
        let r = MergeAuthorizationRestriction::new();

        // Review by the author does NOT count.
        let patch = make_patch_with(vec![approval("author")], "author");
        let old = old_open("author");
        assert!(
            evaluate(&r, &store, patch, Some(old), &user_actor("author"))
                .await
                .is_err()
        );

        // Review by alice counts.
        let patch = make_patch_with(vec![approval("alice")], "author");
        let old = old_open("author");
        evaluate(&r, &store, patch, Some(old), &user_actor("author"))
            .await
            .expect("alice review counts even when author also approved");
    }

    // ---- Layer priority -------------------------------------------------

    #[tokio::test]
    async fn reviews_layer_short_circuits_mergers_evaluation() {
        let store = MemoryStore::new();
        let policy = MergePolicy {
            reviewers: vec![ReviewerGroup {
                label: None,
                any_of: vec![user("reviewer")],
                count: 1,
                exclude_author: true,
            }],
            mergers: Some(MergerRule {
                any_of: vec![user("alice")],
            }),
        };
        add_repo_with_policy(&store, Some(policy)).await;

        let r = MergeAuthorizationRestriction::new();
        // No reviews and the actor is "bob" — both layers would fail if
        // evaluated, but priority gates to "reviews" only.
        let patch = make_patch_with(Vec::new(), "author");
        let old = old_open("author");
        let err = evaluate(&r, &store, patch, Some(old), &user_actor("bob"))
            .await
            .unwrap_err();
        let body = parse_message(&err);
        assert_eq!(body.blocked_at_layer, BlockedAtLayer::Reviews);
        assert!(
            body.reasons
                .iter()
                .all(|r| matches!(r, MergeBlockedReason::MissingApprovals { .. })),
            "should not include NotInMergers when reviews layer is blocked"
        );
    }

    #[tokio::test]
    async fn mergers_layer_reached_after_reviews_satisfied() {
        let store = MemoryStore::new();
        let policy = MergePolicy {
            reviewers: vec![ReviewerGroup {
                label: None,
                any_of: vec![user("reviewer")],
                count: 1,
                exclude_author: true,
            }],
            mergers: Some(MergerRule {
                any_of: vec![user("alice")],
            }),
        };
        add_repo_with_policy(&store, Some(policy)).await;

        let r = MergeAuthorizationRestriction::new();
        let patch = make_patch_with(vec![approval("reviewer")], "author");
        let old = old_open("author");
        let err = evaluate(&r, &store, patch, Some(old), &user_actor("bob"))
            .await
            .unwrap_err();
        let body = parse_message(&err);
        assert_eq!(body.blocked_at_layer, BlockedAtLayer::Mergers);
        assert_eq!(body.reasons.len(), 1);
        assert!(matches!(
            body.reasons[0],
            MergeBlockedReason::NotInMergers { .. }
        ));
    }

    // ---- Status-transition gating --------------------------------------

    #[tokio::test]
    async fn non_merged_update_is_no_op() {
        let store = MemoryStore::new();
        let policy = MergePolicy {
            reviewers: vec![ReviewerGroup {
                label: None,
                any_of: vec![user("reviewer")],
                count: 1,
                exclude_author: true,
            }],
            mergers: None,
        };
        add_repo_with_policy(&store, Some(policy)).await;

        let r = MergeAuthorizationRestriction::new();
        // new.status == Open: restriction is inert even though no reviewers approved.
        let mut patch = make_patch_with(Vec::new(), "author");
        patch.status = PatchStatus::Open;
        let old = old_open("author");
        evaluate(&r, &store, patch, Some(old), &user_actor("anyone"))
            .await
            .expect("restriction must not fire on non-Merged updates");
    }

    #[tokio::test]
    async fn already_merged_update_is_no_op() {
        let store = MemoryStore::new();
        let policy = MergePolicy {
            reviewers: vec![ReviewerGroup {
                label: None,
                any_of: vec![user("reviewer")],
                count: 1,
                exclude_author: true,
            }],
            mergers: None,
        };
        add_repo_with_policy(&store, Some(policy)).await;

        let r = MergeAuthorizationRestriction::new();
        // new and old both Merged: not a transition INTO Merged, so no-op.
        let patch = make_patch_with(Vec::new(), "author");
        let old = make_patch_with(Vec::new(), "author"); // status: Merged
        evaluate(&r, &store, patch, Some(old), &user_actor("anyone"))
            .await
            .expect("Merged -> Merged is not a transition; must not fire");
    }

    #[tokio::test]
    async fn create_patch_operation_is_no_op() {
        let store = MemoryStore::new();
        // No need to add a repo; the operation check short-circuits first.
        let r = MergeAuthorizationRestriction::new();
        let patch = make_patch_with(Vec::new(), "author");
        let payload = OperationPayload::Patch {
            patch_id: None,
            new: patch,
            old: None,
        };
        let actor = user_actor("anyone");
        let ctx = RestrictionContext {
            operation: Operation::CreatePatch,
            actor: &actor,
            payload: &payload,
            store: &store,
        };
        assert!(r.evaluate(&ctx).await.is_ok());
    }

    // ---- JSON wire-shape check -----------------------------------------

    #[tokio::test]
    async fn violation_message_round_trips_through_merge_blocked_error() {
        let store = MemoryStore::new();
        let policy = MergePolicy {
            reviewers: vec![ReviewerGroup {
                label: Some("code-review".to_string()),
                any_of: vec![user("reviewer")],
                count: 1,
                exclude_author: true,
            }],
            mergers: None,
        };
        add_repo_with_policy(&store, Some(policy)).await;

        let r = MergeAuthorizationRestriction::new();
        let patch = make_patch_with(Vec::new(), "author");
        let old = old_open("author");
        let err = evaluate(&r, &store, patch, Some(old), &user_actor("author"))
            .await
            .unwrap_err();

        let body: MergeBlockedError = serde_json::from_str(&err.message)
            .expect("PolicyViolation.message must deserialize as MergeBlockedError");
        assert_eq!(body.blocked_at_layer, BlockedAtLayer::Reviews);
        assert_eq!(body.reasons.len(), 1);
        match &body.reasons[0] {
            MergeBlockedReason::MissingApprovals {
                group_index,
                label,
                needed,
                eligible_principals,
                ..
            } => {
                assert_eq!(*group_index, 0);
                assert_eq!(label.as_deref(), Some("code-review"));
                assert_eq!(*needed, 1);
                assert_eq!(eligible_principals.len(), 1);
            }
            other => panic!("expected MissingApprovals, got {other:?}"),
        }
    }
}
