#[cfg(test)]
pub mod ids {
    use hydra_common::{HydraIdError, IssueId, LabelId, PatchId, SessionId, TriggerId};
    use std::iter;
    use std::str::FromStr;

    pub fn task_id(label: &str) -> SessionId {
        parse_or_build(label, "s-")
    }

    pub fn issue_id(label: &str) -> IssueId {
        parse_or_build(label, "i-")
    }

    pub fn patch_id(label: &str) -> PatchId {
        parse_or_build(label, "p-")
    }

    pub fn label_id(label: &str) -> LabelId {
        parse_or_build(label, "l-")
    }

    pub fn trigger_id(label: &str) -> TriggerId {
        parse_or_build(label, "t-")
    }

    fn parse_or_build<T>(label: &str, prefix: &str) -> T
    where
        T: FromStr<Err = HydraIdError>,
    {
        T::from_str(label).unwrap_or_else(|_| {
            let normalized = format!("{prefix}{}", normalize_suffix(label));
            T::from_str(&normalized).unwrap_or_else(|err| {
                panic!("failed to construct test id from '{label}' (normalized: '{normalized}'): {err}")
            })
        })
    }

    fn normalize_suffix(label: &str) -> String {
        const MIN_LEN: usize = 4;
        const MAX_LEN: usize = 12;

        let mut suffix = String::with_capacity(label.len());
        for ch in label.chars() {
            if ch.is_ascii_alphabetic() {
                suffix.push(ch.to_ascii_lowercase());
            } else if ch.is_ascii_digit() {
                let mapped = (b'a' + (ch as u8 - b'0')) as char;
                suffix.push(mapped);
            } else if ch == '-' || ch == '_' {
                suffix.push('x');
            }
        }

        if suffix.is_empty() {
            suffix.push_str("aaaa");
        }

        if suffix.len() < MIN_LEN {
            suffix.extend(iter::repeat_n('a', MIN_LEN.saturating_sub(suffix.len())));
        }

        if suffix.len() > MAX_LEN {
            suffix.truncate(MAX_LEN);
        }

        suffix
    }
}

#[cfg(test)]
pub mod status {
    use hydra_common::api::v1::projects::{StatusDefinition, StatusKey};
    use hydra_common::Rgb;

    /// Synthesize a neutral [`StatusDefinition`] for tests. Most CLI
    /// tests only assert on identity-level fields (e.g. `status.key`),
    /// so display props default to an empty label and a neutral grey.
    pub fn make_status_def(key: StatusKey) -> StatusDefinition {
        StatusDefinition::new(
            key,
            String::new(),
            Rgb::try_from("#888888".to_string()).expect("well-formed default rgb"),
            false,
            false,
            false,
            None,
        )
    }
}

#[cfg(test)]
pub mod env {
    use std::sync::{Mutex, MutexGuard, OnceLock};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    pub fn lock() -> MutexGuard<'static, ()> {
        ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("env lock poisoned")
    }
}
