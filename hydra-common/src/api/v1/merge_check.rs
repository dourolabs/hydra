use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use crate::PatchId;
use crate::api::v1::agents::AgentName;
use crate::api::v1::repositories::DynamicRef;
use crate::api::v1::users::Username;
use crate::principal::ExternalSystem;

// `DynamicRef` already (de)serialises as the `@patch.author` shorthand the
// design assumes (see `hydra-common/src/api/v1/repositories.rs`). We reuse it
// verbatim for the `ref` field of [`EligiblePrincipal::Dynamic`] rather than
// introducing a parallel enum.

/// Response body of `POST /v1/patches/:id/merge_check`.
///
/// On success the server returns `{ "ok": true }`; on a blocked merge the
/// body is the same [`MergeBlockedError`] payload the write-path returns.
/// Untagged so the wire JSON is exactly one of those two shapes — no
/// discriminator wrapper.
///
/// HTTP status: `200` for [`MergeCheckResponse::Ok`], `422` for
/// [`MergeCheckResponse::Blocked`]. The 422 (Unprocessable Entity) choice
/// is locked in by the `IntoResponse` impl and is documented in
/// `/designs/merge-time-constraints.md` §4.3 / §4.5 — it is NOT 400
/// (request well-formed) and NOT 403 (actor authorisation is only one of
/// two layers).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(untagged)]
pub enum MergeCheckResponse {
    Ok(MergeCheckOk),
    Blocked(MergeBlockedError),
}

/// Success body of `POST /v1/patches/:id/merge_check`. The `ok` field is
/// always `true` — its purpose is to give the variant a unique JSON shape
/// so the untagged [`MergeCheckResponse`] can be parsed without ambiguity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct MergeCheckOk {
    pub ok: bool,
}

impl MergeCheckOk {
    pub fn allowed() -> Self {
        Self { ok: true }
    }
}

impl IntoResponse for MergeCheckResponse {
    fn into_response(self) -> Response {
        match self {
            Self::Ok(body) => (StatusCode::OK, Json(body)).into_response(),
            Self::Blocked(body) => (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response(),
        }
    }
}

/// Structured body of a `merge_blocked` response.
///
/// This wire shape is carried verbatim across three surfaces (see
/// `/designs/merge-time-constraints.md` §4.5):
///
/// - the HTTP body of `POST /v1/patches/:id/merge_check` (Phase 2 PR-3),
/// - serialised into the `message` field of the `PolicyViolation` emitted by
///   the `merge_authorization` restriction (Phase 2 PR-2),
/// - parsed by `hydra patches merge --json` (Phase 2 PR-4).
///
/// `reasons` always contains failures from EXACTLY ONE layer — the
/// highest-priority unsatisfied layer named by `blocked_at_layer`. See §4.5
/// for the priority ordering and the rationale.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct MergeBlockedError {
    pub code: MergeBlockedCode,
    pub patch_id: PatchId,
    pub blocked_at_layer: BlockedAtLayer,
    pub reasons: Vec<MergeBlockedReason>,
}

/// Discriminator for the error envelope. Currently a single variant; the enum
/// exists so future error categories can be added without changing the wire
/// shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(rename_all = "snake_case")]
pub enum MergeBlockedCode {
    MergeBlocked,
}

/// Which authorisation layer produced the failures in `reasons`. New layers
/// (e.g. a future server-side merge action) can be added without breaking
/// existing clients; the SWE dispatches on this field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(rename_all = "snake_case")]
pub enum BlockedAtLayer {
    Reviews,
    Mergers,
}

/// One reason a merge attempt was blocked. Internally tagged on `kind` so new
/// reason variants can be added without breaking parsers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MergeBlockedReason {
    /// A reviewer group is short of the required approving principals.
    MissingApprovals {
        group_index: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        eligible_principals: Vec<EligiblePrincipal>,
        current_approvals: Vec<String>,
        needed: u32,
        suggested_action: SuggestedAction,
    },
    /// The acting actor is not present in the repo's `mergers.any_of`.
    NotInMergers {
        actor: String,
        allowed_mergers: Vec<EligiblePrincipal>,
        suggested_action: SuggestedAction,
    },
}

/// A resolved principal as it appears in a merge_blocked error.
///
/// Internally tagged on `kind` so future principal forms (e.g. a `group`
/// variant if a team concept lands) can be added without breaking older
/// parsers.
///
/// The three static variants (`User`, `Agent`, `External`) mirror the
/// shared [`crate::Principal`] type so consumers can route follow-up
/// actions by kind — a `MergeBlockedError` referencing `agents/swe`
/// surfaces as `EligiblePrincipal::Agent { name: "swe" }`, not a bare
/// string that could be confused with a same-named user.
///
/// `Dynamic` always carries both `ref` (the raw `@…` shorthand from the
/// policy, reusing the existing [`DynamicRef`] wire form) and `resolved_to`
/// (the username it resolves to right now, or `null` if it could not be
/// resolved — e.g. the patch has no parent issue).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EligiblePrincipal {
    User {
        name: Username,
    },
    Agent {
        name: AgentName,
    },
    External {
        system: ExternalSystem,
        username: String,
    },
    Dynamic {
        #[serde(rename = "ref")]
        reference: DynamicRef,
        resolved_to: Option<String>,
    },
}

/// Non-binding hint for the SWE / CLI: which kind of follow-up issue to file
/// in response to a particular reason, and who to assign it to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SuggestedAction {
    FileReviewRequest {
        assign_to_one_of: Vec<String>,
        title_hint: String,
    },
    FileMergeRequest {
        assign_to_one_of: Vec<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn patch_id() -> PatchId {
        // Suffix must be all-alphabetic and at least MIN_RANDOM_LEN (4) chars.
        PatchId::try_from("p-xyzabc".to_string()).expect("valid patch id")
    }

    fn reviews_blocked_value() -> serde_json::Value {
        json!({
            "code": "merge_blocked",
            "patch_id": "p-xyzabc",
            "blocked_at_layer": "reviews",
            "reasons": [
                {
                    "kind": "missing_approvals",
                    "group_index": 0,
                    "label": "code-review",
                    "eligible_principals": [
                        { "kind": "user", "name": "reviewer" },
                        { "kind": "agent", "name": "swe" },
                        { "kind": "external", "system": "github", "username": "jayantk" }
                    ],
                    "current_approvals": [],
                    "needed": 1,
                    "suggested_action": {
                        "kind": "file_review_request",
                        "assign_to_one_of": ["reviewer", "swe", "jayantk"],
                        "title_hint": "Review p-xyzabc (code-review)"
                    }
                }
            ]
        })
    }

    fn mergers_blocked_value() -> serde_json::Value {
        json!({
            "code": "merge_blocked",
            "patch_id": "p-xyzabc",
            "blocked_at_layer": "mergers",
            "reasons": [
                {
                    "kind": "not_in_mergers",
                    "actor": "swe-session-abcd",
                    "allowed_mergers": [
                        {
                            "kind": "dynamic",
                            "ref": "@patch.author",
                            "resolved_to": "jayantk"
                        }
                    ],
                    "suggested_action": {
                        "kind": "file_merge_request",
                        "assign_to_one_of": ["jayantk"]
                    }
                }
            ]
        })
    }

    #[test]
    fn reviews_blocked_body_round_trips_byte_for_byte() {
        let expected = reviews_blocked_value();
        let parsed: MergeBlockedError = serde_json::from_value(expected.clone()).unwrap();
        let reserialised = serde_json::to_value(&parsed).unwrap();
        assert_eq!(reserialised, expected);
    }

    #[test]
    fn mergers_blocked_body_round_trips_byte_for_byte() {
        let expected = mergers_blocked_value();
        let parsed: MergeBlockedError = serde_json::from_value(expected.clone()).unwrap();
        let reserialised = serde_json::to_value(&parsed).unwrap();
        assert_eq!(reserialised, expected);
    }

    #[test]
    fn parsed_reviews_blocked_body_matches_expected_value() {
        let parsed: MergeBlockedError = serde_json::from_value(reviews_blocked_value()).unwrap();
        assert_eq!(parsed.code, MergeBlockedCode::MergeBlocked);
        assert_eq!(parsed.patch_id, patch_id());
        assert_eq!(parsed.blocked_at_layer, BlockedAtLayer::Reviews);
        assert_eq!(parsed.reasons.len(), 1);
        match &parsed.reasons[0] {
            MergeBlockedReason::MissingApprovals {
                group_index,
                label,
                eligible_principals,
                current_approvals,
                needed,
                suggested_action,
            } => {
                assert_eq!(*group_index, 0);
                assert_eq!(label.as_deref(), Some("code-review"));
                assert_eq!(eligible_principals.len(), 3);
                assert_eq!(
                    eligible_principals[0],
                    EligiblePrincipal::User {
                        name: Username::try_new("reviewer").unwrap(),
                    }
                );
                assert_eq!(
                    eligible_principals[1],
                    EligiblePrincipal::Agent {
                        name: AgentName::try_new("swe").unwrap(),
                    }
                );
                assert_eq!(
                    eligible_principals[2],
                    EligiblePrincipal::External {
                        system: ExternalSystem::try_new("github").unwrap(),
                        username: "jayantk".to_string(),
                    }
                );
                assert!(current_approvals.is_empty());
                assert_eq!(*needed, 1);
                match suggested_action {
                    SuggestedAction::FileReviewRequest {
                        assign_to_one_of,
                        title_hint,
                    } => {
                        assert_eq!(assign_to_one_of, &vec!["reviewer", "swe", "jayantk"]);
                        assert_eq!(title_hint, "Review p-xyzabc (code-review)");
                    }
                    other => panic!("expected file_review_request, got {other:?}"),
                }
            }
            other => panic!("expected missing_approvals, got {other:?}"),
        }
    }

    #[test]
    fn parsed_mergers_blocked_body_matches_expected_value() {
        let parsed: MergeBlockedError = serde_json::from_value(mergers_blocked_value()).unwrap();
        assert_eq!(parsed.blocked_at_layer, BlockedAtLayer::Mergers);
        match &parsed.reasons[0] {
            MergeBlockedReason::NotInMergers {
                actor,
                allowed_mergers,
                suggested_action,
            } => {
                assert_eq!(actor, "swe-session-abcd");
                assert_eq!(
                    allowed_mergers,
                    &vec![EligiblePrincipal::Dynamic {
                        reference: DynamicRef::PatchAuthor,
                        resolved_to: Some("jayantk".to_string()),
                    }]
                );
                match suggested_action {
                    SuggestedAction::FileMergeRequest { assign_to_one_of } => {
                        assert_eq!(assign_to_one_of, &vec!["jayantk"]);
                    }
                    other => panic!("expected file_merge_request, got {other:?}"),
                }
            }
            other => panic!("expected not_in_mergers, got {other:?}"),
        }
    }

    #[test]
    fn dynamic_principal_with_unresolved_ref_serialises_as_null() {
        let principal = EligiblePrincipal::Dynamic {
            reference: DynamicRef::PatchAuthor,
            resolved_to: None,
        };
        let value = serde_json::to_value(&principal).unwrap();
        assert_eq!(
            value,
            json!({
                "kind": "dynamic",
                "ref": "@patch.author",
                "resolved_to": null,
            })
        );
        let back: EligiblePrincipal = serde_json::from_value(value).unwrap();
        assert_eq!(back, principal);
    }

    #[test]
    fn missing_approvals_omits_label_when_none() {
        // A group without a label round-trips with the field absent (not null).
        let reason = MergeBlockedReason::MissingApprovals {
            group_index: 2,
            label: None,
            eligible_principals: vec![EligiblePrincipal::User {
                name: Username::try_new("alice").unwrap(),
            }],
            current_approvals: vec![],
            needed: 1,
            suggested_action: SuggestedAction::FileReviewRequest {
                assign_to_one_of: vec!["alice".to_string()],
                title_hint: "Review p-xyzabc".to_string(),
            },
        };
        let value = serde_json::to_value(&reason).unwrap();
        assert!(
            !value.as_object().unwrap().contains_key("label"),
            "label should be omitted when None, got {value}"
        );
        let back: MergeBlockedReason = serde_json::from_value(value).unwrap();
        assert_eq!(back, reason);
    }

    #[test]
    fn unknown_code_is_rejected() {
        let value = json!({
            "code": "totally_made_up",
            "patch_id": "p-xyzabc",
            "blocked_at_layer": "reviews",
            "reasons": [],
        });
        let err = serde_json::from_value::<MergeBlockedError>(value).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("totally_made_up") || msg.contains("merge_blocked"),
            "unknown code error should mention the offending value or the accepted set, got: {msg}"
        );
    }

    #[test]
    fn unknown_blocked_at_layer_is_rejected() {
        let value = json!({
            "code": "merge_blocked",
            "patch_id": "p-xyzabc",
            "blocked_at_layer": "atomic_kittens",
            "reasons": [],
        });
        let err = serde_json::from_value::<MergeBlockedError>(value).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("atomic_kittens") || msg.contains("reviews"),
            "unknown blocked_at_layer error should mention the offending value or accepted set, got: {msg}"
        );
    }

    #[test]
    fn unknown_reason_kind_is_rejected() {
        let value = json!({
            "code": "merge_blocked",
            "patch_id": "p-xyzabc",
            "blocked_at_layer": "reviews",
            "reasons": [
                { "kind": "ufo_landed" }
            ],
        });
        let err = serde_json::from_value::<MergeBlockedError>(value).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("ufo_landed") || msg.contains("missing_approvals"),
            "unknown reason kind error should mention the offending value or accepted set, got: {msg}"
        );
    }

    #[test]
    fn unknown_eligible_principal_kind_is_rejected() {
        let value = json!({
            "kind": "robot",
            "id": "r2d2",
        });
        let err = serde_json::from_value::<EligiblePrincipal>(value).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("robot") || msg.contains("user"),
            "unknown EligiblePrincipal kind error should mention the offending value or accepted set, got: {msg}"
        );
    }

    // ---- MergeCheckResponse round-trips ---------------------------------

    #[test]
    fn merge_check_response_ok_round_trips() {
        let value = json!({ "ok": true });
        let parsed: MergeCheckResponse = serde_json::from_value(value.clone()).unwrap();
        assert!(matches!(
            parsed,
            MergeCheckResponse::Ok(MergeCheckOk { ok: true })
        ));
        let reserialised = serde_json::to_value(&parsed).unwrap();
        assert_eq!(reserialised, value);
    }

    #[test]
    fn merge_check_response_blocked_round_trips() {
        let value = reviews_blocked_value();
        let parsed: MergeCheckResponse = serde_json::from_value(value.clone()).unwrap();
        assert!(matches!(parsed, MergeCheckResponse::Blocked(_)));
        let reserialised = serde_json::to_value(&parsed).unwrap();
        assert_eq!(reserialised, value);
    }

    #[test]
    fn merge_check_response_blocked_does_not_swallow_ok_field() {
        // Defensive: a blocked body must NEVER be confused with the ok variant
        // by the untagged enum, even though serde tries variants in source
        // order. The `code` / `patch_id` / `blocked_at_layer` / `reasons`
        // fields are required by MergeBlockedError; `ok` is required by
        // MergeCheckOk. The two shapes are disjoint.
        let value = mergers_blocked_value();
        let parsed: MergeCheckResponse = serde_json::from_value(value).unwrap();
        match parsed {
            MergeCheckResponse::Blocked(body) => {
                assert_eq!(body.blocked_at_layer, BlockedAtLayer::Mergers);
            }
            MergeCheckResponse::Ok(_) => panic!("must not match Ok"),
        }
    }

    #[test]
    fn unknown_suggested_action_kind_is_rejected() {
        let value = json!({
            "kind": "send_pigeon",
            "assign_to_one_of": ["alice"],
        });
        let err = serde_json::from_value::<SuggestedAction>(value).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("send_pigeon") || msg.contains("file_"),
            "unknown SuggestedAction kind error should mention the offending value or accepted set, got: {msg}"
        );
    }
}
