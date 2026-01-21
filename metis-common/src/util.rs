use std::env;

/// RAII guard that applies temporary environment variable overrides and restores
/// previous values on drop.
pub struct EnvGuard {
    previous: Vec<(String, Option<String>)>,
}

impl EnvGuard {
    pub fn set(pairs: &[(&str, Option<&str>)]) -> EnvGuard {
        let mut previous = Vec::with_capacity(pairs.len());
        for (key, value) in pairs {
            let key_string = key.to_string();
            let original = env::var(key).ok();
            match value {
                // SAFETY: Process-wide environment mutation is required here; the guard restores
                // the prior values on drop to limit the unsafety to this scope.
                Some(new_value) => unsafe { env::set_var(key, new_value) },
                // SAFETY: See above.
                None => unsafe { env::remove_var(key) },
            }
            previous.push((key_string, original));
        }
        EnvGuard { previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, original) in self.previous.drain(..) {
            if let Some(value) = original {
                // SAFETY: Restores the environment to the state captured in `set`.
                unsafe { env::set_var(key, value) };
            } else {
                // SAFETY: Restores the environment to the state captured in `set`.
                unsafe { env::remove_var(key) };
            }
        }
    }
}
