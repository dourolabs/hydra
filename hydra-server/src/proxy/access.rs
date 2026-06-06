//! Read-access checks for the proxy.
//!
//! Per `d-lhiauhk` §"Reach authorization", the same authority required
//! to read a session/conversation transcript governs proxy reach into
//! that target. This is enforced **at every proxy request**, not just
//! at cookie mint, so revoking a principal's membership invalidates
//! open browser tabs at the next request rather than at cookie expiry.

use hydra_common::actor_ref::ActorId;
use hydra_common::api::v1::users::Username;

use crate::app::AppState;
use crate::domain::sessions::{Session, SessionMode};

use super::cookie::ProxyTargetId;

/// Return the principal `Username` iff `actor_id` is a User actor.
///
/// All `Session.creator` / `Conversation.creator` values are `Username`s,
/// so the non-User variants (Agent, Adhoc, External) can never satisfy a
/// creator-match and may as well be rejected at the type level. This
/// avoids the implicit cross-type string comparison the previous helper
/// did across every `ActorId` variant.
pub fn user_principal(actor_id: &ActorId) -> Option<&Username> {
    match actor_id {
        ActorId::User(u) => Some(u),
        _ => None,
    }
}

/// Returns `true` iff `actor_id` still has read access to `target`.
///
/// Mirrors the rules enforced at cookie mint:
///   - `Session(_)`: actor must be the session's creator, or — for an
///     Interactive session — the owning conversation's creator.
///   - `Conversation(_)`: actor must be the conversation's creator.
///
/// The caller passes the already-loaded `session` for the resolved
/// target (every proxy request loads it for the port-allowlist check
/// regardless, so this avoids a redundant store hit). Errors loading
/// the owning conversation count as "no access" — defaulting to deny
/// when we can't confirm the membership.
pub async fn has_read_access(
    state: &AppState,
    actor_id: &ActorId,
    target: &ProxyTargetId,
    session: &Session,
) -> bool {
    let Some(principal) = user_principal(actor_id) else {
        return false;
    };
    // `Session.creator` and `Conversation.creator` are `domain::users::Username`,
    // whereas the cookie payload's `ActorId::User(_)` carries
    // `hydra_common::users::Username`. The two wrap the same string but
    // are distinct types, so compare by `as_str()`.
    let principal = principal.as_str();

    match target {
        ProxyTargetId::Session(_) => {
            if session.creator.as_str() == principal {
                return true;
            }
            if let SessionMode::Interactive {
                conversation_id, ..
            } = &session.mode
            {
                if let Ok(versioned) = state.store().get_conversation(conversation_id, false).await
                {
                    return versioned.item.creator.as_str() == principal;
                }
            }
            false
        }
        ProxyTargetId::Conversation(cid) => {
            match state.store().get_conversation(cid, false).await {
                Ok(versioned) => versioned.item.creator.as_str() == principal,
                Err(_) => false,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hydra_common::SessionId;

    #[test]
    fn user_principal_returns_some_for_user_actor() {
        let username = Username::from("alice");
        let actor = ActorId::User(username.clone());
        assert_eq!(user_principal(&actor), Some(&username));
    }

    #[test]
    fn user_principal_returns_none_for_agent_actor() {
        let agent = hydra_common::api::v1::agents::AgentName::try_new("swe").unwrap();
        let actor = ActorId::Agent(agent);
        assert!(user_principal(&actor).is_none());
    }

    #[test]
    fn user_principal_returns_none_for_adhoc_actor() {
        let actor = ActorId::Adhoc(SessionId::new());
        assert!(user_principal(&actor).is_none());
    }
}
