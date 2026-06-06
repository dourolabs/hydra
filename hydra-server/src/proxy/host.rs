//! Host-label parser for the proxy subdomain.
//!
//! Recognized form: `<digits>-<HydraId>.proxy.<host>` where `<HydraId>`
//! is a session id (`s-…`) or conversation id (`c-…`).
//!
//! The parser:
//! - strips the port off the `Host` header (`example.com:8080` → `example.com`),
//! - verifies the `.proxy.<host>` suffix (`host` is the operator's
//!   configured `proxy_host`),
//! - splits the leading label at the FIRST `-` (so HydraId suffixes,
//!   which contain `-` after their prefix, stay intact),
//! - validates the port is in [1, 65535] and the id parses to a
//!   `SessionId` or `ConversationId`.
//!
//! Returns `Err(HostParseError)` for any malformed input so the
//! subdomain router can reject with `400 Bad Request`.

use hydra_common::{ConversationId, SessionId};

use super::cookie::ProxyTargetId;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HostParseError {
    #[error("missing Host header")]
    Missing,
    #[error("host '{host}' does not end with the proxy suffix '.{suffix}'")]
    SuffixMismatch { host: String, suffix: String },
    #[error("proxy label '{label}' is not `<port>-<id>` form")]
    BadLabel { label: String },
    #[error("proxy label '{label}' has a non-numeric port")]
    BadPort { label: String },
    #[error("proxy label '{label}' has an unrecognized HydraId prefix")]
    BadId { label: String },
}

/// Parsed proxy-subdomain host label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedProxyHost {
    pub port: u16,
    pub target: ProxyTargetId,
}

/// Parse a `Host` header value into `(port, target)`.
///
/// `proxy_suffix` is the operator-configured suffix without a leading dot
/// (e.g. `"proxy.example.com"`); the parser rejects hosts that don't end
/// with `.<suffix>`.
pub fn parse(host_header: &str, proxy_suffix: &str) -> Result<ParsedProxyHost, HostParseError> {
    if host_header.is_empty() {
        return Err(HostParseError::Missing);
    }

    // Strip a `:port` suffix if present. Hosts cannot contain `:` other
    // than for the port (we don't carry IPv6 literals on the proxy
    // subdomain).
    let host_no_port = host_header
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or(host_header);

    let dotted_suffix = format!(".{proxy_suffix}");
    let label = host_no_port.strip_suffix(&dotted_suffix).ok_or_else(|| {
        HostParseError::SuffixMismatch {
            host: host_no_port.to_string(),
            suffix: proxy_suffix.to_string(),
        }
    })?;

    // The label must not itself contain a `.` — otherwise a host like
    // `foo.3000-c-abc.proxy.host` would slip through.
    if label.contains('.') {
        return Err(HostParseError::BadLabel {
            label: label.to_string(),
        });
    }

    // Split the leading label at the FIRST `-`. The HydraId's own internal
    // `-` (e.g. `c-abc1234`) lives in the second half.
    let (port_str, id_str) = label
        .split_once('-')
        .ok_or_else(|| HostParseError::BadLabel {
            label: label.to_string(),
        })?;

    let port: u16 = port_str.parse().map_err(|_| HostParseError::BadPort {
        label: label.to_string(),
    })?;
    if port == 0 {
        return Err(HostParseError::BadPort {
            label: label.to_string(),
        });
    }

    if let Ok(s) = id_str.parse::<SessionId>() {
        return Ok(ParsedProxyHost {
            port,
            target: ProxyTargetId::Session(s),
        });
    }
    if let Ok(c) = id_str.parse::<ConversationId>() {
        return Ok(ParsedProxyHost {
            port,
            target: ProxyTargetId::Conversation(c),
        });
    }

    Err(HostParseError::BadId {
        label: label.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    const PROXY_SUFFIX: &str = "proxy.localhost";

    fn s_id() -> SessionId {
        SessionId::from_str("s-abcdef").unwrap()
    }
    fn c_id() -> ConversationId {
        ConversationId::from_str("c-abcdef").unwrap()
    }

    #[test]
    fn parses_session_label() {
        let host = format!("3000-{}.{}", s_id(), PROXY_SUFFIX);
        let parsed = parse(&host, PROXY_SUFFIX).unwrap();
        assert_eq!(parsed.port, 3000);
        assert_eq!(parsed.target, ProxyTargetId::Session(s_id()));
    }

    #[test]
    fn parses_conversation_label() {
        let host = format!("3000-{}.{}", c_id(), PROXY_SUFFIX);
        let parsed = parse(&host, PROXY_SUFFIX).unwrap();
        assert_eq!(parsed.port, 3000);
        assert_eq!(parsed.target, ProxyTargetId::Conversation(c_id()));
    }

    #[test]
    fn strips_port_from_host_header() {
        let host = format!("3000-{}.{}:8080", c_id(), PROXY_SUFFIX);
        let parsed = parse(&host, PROXY_SUFFIX).unwrap();
        assert_eq!(parsed.port, 3000);
    }

    #[test]
    fn rejects_missing_port_segment() {
        // `foo.proxy.host` — no `port-id` label.
        let host = format!("foo.{PROXY_SUFFIX}");
        assert!(matches!(
            parse(&host, PROXY_SUFFIX),
            Err(HostParseError::BadLabel { .. })
        ));
    }

    #[test]
    fn rejects_unrecognized_id_prefix() {
        // `3000-x-bad.proxy.host` — `x-` isn't a HydraId prefix.
        let host = format!("3000-x-bad.{PROXY_SUFFIX}");
        assert!(matches!(
            parse(&host, PROXY_SUFFIX),
            Err(HostParseError::BadId { .. })
        ));
    }

    #[test]
    fn rejects_wrong_separator() {
        // `3000.c-abc.proxy.host` — wrong separator (`.` instead of `-`).
        let host = format!("3000.{}.{PROXY_SUFFIX}", c_id());
        assert!(matches!(
            parse(&host, PROXY_SUFFIX),
            Err(HostParseError::BadLabel { .. })
        ));
    }

    #[test]
    fn rejects_missing_proxy_suffix() {
        let host = format!("3000-{}.other.example.com", c_id());
        assert!(matches!(
            parse(&host, PROXY_SUFFIX),
            Err(HostParseError::SuffixMismatch { .. })
        ));
    }

    #[test]
    fn rejects_empty_host() {
        assert!(matches!(
            parse("", PROXY_SUFFIX),
            Err(HostParseError::Missing)
        ));
    }

    #[test]
    fn rejects_port_zero() {
        let host = format!("0-{}.{PROXY_SUFFIX}", c_id());
        assert!(matches!(
            parse(&host, PROXY_SUFFIX),
            Err(HostParseError::BadPort { .. })
        ));
    }

    #[test]
    fn rejects_port_overflow() {
        let host = format!("70000-{}.{PROXY_SUFFIX}", c_id());
        assert!(matches!(
            parse(&host, PROXY_SUFFIX),
            Err(HostParseError::BadPort { .. })
        ));
    }
}
