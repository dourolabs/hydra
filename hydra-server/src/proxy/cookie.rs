//! Proxy cookie sign/validate.
//!
//! The proxy subdomain authenticates requests via a cookie minted by the
//! main API. The cookie value is `base64url(secret_manager.encrypt(json))`
//! where `json` is a [`ProxyCookiePayload`] binding
//! `(actor_id, target, session_id_at_mint, exp)`. AES-GCM provides both
//! confidentiality and integrity, so a tampered value fails decryption
//! cleanly.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hydra_common::actor_ref::ActorId;
use hydra_common::{ConversationId, SessionId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::secrets::{SecretManager, SecretManagerError};

/// The proxy target — either a session id (direct reach) or a conversation id
/// (active-session reach).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum ProxyTargetId {
    Session(SessionId),
    Conversation(ConversationId),
}

impl ProxyTargetId {
    pub fn as_label(&self) -> &str {
        match self {
            ProxyTargetId::Session(s) => s.as_ref(),
            ProxyTargetId::Conversation(c) => c.as_ref(),
        }
    }
}

impl std::fmt::Display for ProxyTargetId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_label())
    }
}

/// Encrypted payload that lives inside the proxy cookie.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyCookiePayload {
    /// The principal authorized to reach the target. Re-validated at every
    /// proxy request against the live read-access rules so revoking the
    /// principal's access invalidates open tabs at the next request.
    pub actor_id: ActorId,
    /// The conversation or session this cookie is scoped to.
    pub target: ProxyTargetId,
    /// The active session id at mint time. The proxy router rejects the
    /// cookie (401) when the target's current active session no longer
    /// matches this — covering the "conversation re-activated against a
    /// new pod" case.
    pub session_id_at_mint: SessionId,
    /// Unix epoch seconds at which the cookie expires.
    pub exp: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum CookieError {
    #[error("cookie payload is not valid base64url")]
    Base64(#[from] base64::DecodeError),
    #[error("cookie payload failed integrity check or decryption")]
    Decrypt(SecretManagerError),
    #[error("cookie payload is not valid JSON")]
    Json(#[from] serde_json::Error),
    #[error("cookie has expired")]
    Expired,
    #[error("cookie binds a different target than the request")]
    TargetMismatch,
    #[error("cookie binds a different session_id_at_mint than the active session")]
    SessionMismatch,
    #[error("failed to encrypt cookie payload: {0}")]
    Encrypt(SecretManagerError),
}

/// Default lifetime for newly-minted proxy cookies.
pub const DEFAULT_COOKIE_TTL_SECS: i64 = 3600;

/// Encrypt + base64url-encode a payload for storage in the cookie value.
pub fn mint(
    secret_manager: &SecretManager,
    payload: &ProxyCookiePayload,
) -> Result<String, CookieError> {
    let json = serde_json::to_vec(payload)?;
    let blob = secret_manager
        .encrypt(
            std::str::from_utf8(&json)
                .expect("serde_json::to_vec always yields valid UTF-8 for a struct payload"),
        )
        .map_err(CookieError::Encrypt)?;
    Ok(URL_SAFE_NO_PAD.encode(blob))
}

/// Decode + decrypt a cookie value to its payload. Does NOT check
/// `exp`/`target`/`session_id_at_mint` — the caller validates those against
/// the host label and current active session.
pub fn decode(
    secret_manager: &SecretManager,
    cookie_value: &str,
) -> Result<ProxyCookiePayload, CookieError> {
    let blob = URL_SAFE_NO_PAD.decode(cookie_value)?;
    let plaintext = secret_manager
        .decrypt(&blob)
        .map_err(CookieError::Decrypt)?;
    let payload: ProxyCookiePayload = serde_json::from_str(&plaintext)?;
    Ok(payload)
}

/// Validate a cookie payload against the host label and the currently
/// active session for the target.
///
/// `now_unix_secs` is taken as a parameter so tests can drive specific
/// time points; production callers pass `chrono::Utc::now().timestamp()`.
pub fn validate(
    payload: &ProxyCookiePayload,
    expected_target: &ProxyTargetId,
    current_active_session: &SessionId,
    now_unix_secs: i64,
) -> Result<(), CookieError> {
    if payload.exp <= now_unix_secs {
        return Err(CookieError::Expired);
    }
    if payload.target != *expected_target {
        return Err(CookieError::TargetMismatch);
    }
    if payload.session_id_at_mint != *current_active_session {
        return Err(CookieError::SessionMismatch);
    }
    Ok(())
}

/// Cookie name for a given target. Includes a short hash of the target id
/// so a browser can hold simultaneous grants for multiple targets without
/// collision: `hydra_proxy_<short>`.
pub fn cookie_name(target: &ProxyTargetId) -> String {
    let mut hasher = Sha256::new();
    hasher.update(target.as_label().as_bytes());
    let digest = hasher.finalize();
    let short = URL_SAFE_NO_PAD.encode(&digest[..6]);
    // Replace base64url chars that are technically valid in cookie names but
    // visually confusing.
    let sanitized: String = short
        .chars()
        .map(|c| match c {
            '-' | '_' => '0',
            other => other,
        })
        .collect();
    format!("hydra_proxy_{sanitized}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use hydra_common::api::v1::users::Username;

    fn make_secret_manager() -> SecretManager {
        SecretManager::new([7u8; 32])
    }

    fn sample_payload() -> ProxyCookiePayload {
        ProxyCookiePayload {
            actor_id: ActorId::User(Username::from("alice")),
            target: ProxyTargetId::Conversation(ConversationId::new()),
            session_id_at_mint: SessionId::new(),
            exp: 9_000_000_000, // far in the future
        }
    }

    #[test]
    fn round_trip_mint_decode() {
        let mgr = make_secret_manager();
        let payload = sample_payload();
        let encoded = mint(&mgr, &payload).unwrap();
        let decoded = decode(&mgr, &encoded).unwrap();
        assert_eq!(payload, decoded);
    }

    #[test]
    fn tampered_cookie_fails_to_decrypt() {
        let mgr = make_secret_manager();
        let payload = sample_payload();
        let encoded = mint(&mgr, &payload).unwrap();
        // Flip a single byte at a known offset that doesn't break the
        // base64 alphabet — replace the last char with a different one
        // from the URL_SAFE_NO_PAD set.
        let mut tampered = encoded.clone();
        let last = tampered.pop().unwrap();
        let swap = if last == 'A' { 'B' } else { 'A' };
        tampered.push(swap);
        assert!(matches!(
            decode(&mgr, &tampered),
            Err(CookieError::Decrypt(_)) | Err(CookieError::Base64(_))
        ));
    }

    #[test]
    fn cookie_from_different_key_fails() {
        let mgr1 = SecretManager::new([1u8; 32]);
        let mgr2 = SecretManager::new([2u8; 32]);
        let payload = sample_payload();
        let encoded = mint(&mgr1, &payload).unwrap();
        assert!(matches!(
            decode(&mgr2, &encoded),
            Err(CookieError::Decrypt(_))
        ));
    }

    #[test]
    fn validate_rejects_expired_cookie() {
        let payload = ProxyCookiePayload {
            exp: 1_000_000,
            ..sample_payload()
        };
        let result = validate(
            &payload,
            &payload.target,
            &payload.session_id_at_mint,
            2_000_000,
        );
        assert!(matches!(result, Err(CookieError::Expired)));
    }

    #[test]
    fn validate_rejects_target_mismatch() {
        let payload = sample_payload();
        let other = ProxyTargetId::Conversation(ConversationId::new());
        let result = validate(&payload, &other, &payload.session_id_at_mint, 1);
        assert!(matches!(result, Err(CookieError::TargetMismatch)));
    }

    #[test]
    fn validate_rejects_session_mismatch() {
        let payload = sample_payload();
        let other_sid = SessionId::new();
        let result = validate(&payload, &payload.target, &other_sid, 1);
        assert!(matches!(result, Err(CookieError::SessionMismatch)));
    }

    #[test]
    fn validate_accepts_matching_payload() {
        let payload = sample_payload();
        validate(&payload, &payload.target, &payload.session_id_at_mint, 1).unwrap();
    }

    #[test]
    fn cookie_name_is_stable_per_target() {
        let target = ProxyTargetId::Conversation(ConversationId::new());
        assert_eq!(cookie_name(&target), cookie_name(&target));
    }

    #[test]
    fn cookie_name_distinct_for_distinct_targets() {
        let a = ProxyTargetId::Conversation(ConversationId::new());
        let b = ProxyTargetId::Conversation(ConversationId::new());
        assert_ne!(cookie_name(&a), cookie_name(&b));
    }

    #[test]
    fn cookie_name_has_hydra_proxy_prefix() {
        let target = ProxyTargetId::Session(SessionId::new());
        let name = cookie_name(&target);
        assert!(name.starts_with("hydra_proxy_"), "got: {name}");
        // No dashes/underscores past the prefix; cookie names with `-` are
        // legal but easy to confuse with the host-label separator.
        let suffix = name.trim_start_matches("hydra_proxy_");
        assert!(!suffix.contains('-'));
    }
}
