//! Shared default color palette and palette-hash helper used by labels and
//! per-project status definitions. Both code paths default a missing color
//! to a deterministic palette entry derived from the entity's name/key.
//!
//! Moved out of `app/labels.rs` per the per-project-statuses design (§4
//! "Frontend display") so that label and status code do not depend on each
//! other.

use hydra_common::Rgb;

/// Default color palette for labels and statuses that don't specify a color.
pub const DEFAULT_COLORS: &[&str] = &[
    "#e74c3c", // red
    "#e67e22", // orange
    "#f1c40f", // yellow
    "#2ecc71", // green
    "#1abc9c", // teal
    "#3498db", // blue
    "#9b59b6", // purple
    "#e91e63", // pink
    "#795548", // brown
    "#607d8b", // blue grey
];

/// Deterministically pick a palette entry for `name` by hashing it into
/// [`DEFAULT_COLORS`]. Same input → same color.
pub fn default_color_for_name(name: &str) -> Rgb {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    let idx = (hasher.finish() as usize) % DEFAULT_COLORS.len();
    DEFAULT_COLORS[idx]
        .parse()
        .expect("DEFAULT_COLORS entries are valid hex colors")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn same_name_yields_same_color() {
        assert_eq!(default_color_for_name("bug"), default_color_for_name("bug"));
        assert_eq!(
            default_color_for_name("in-progress"),
            default_color_for_name("in-progress")
        );
    }

    #[test]
    fn output_is_a_palette_entry() {
        let color = default_color_for_name("feature");
        assert!(DEFAULT_COLORS.contains(&color.as_ref()));
    }

    #[test]
    fn hash_distribution_covers_palette() {
        // Hash a wide set of names and confirm we hit at least half of the
        // palette buckets. This is a smoke test — DefaultHasher is the
        // standard library's default, not a property-tested PRF, but a
        // distribution this weak would still indicate a regression in the
        // helper.
        let mut hit: HashSet<String> = HashSet::new();
        for n in 0..200 {
            let name = format!("name-{n}");
            hit.insert(default_color_for_name(&name).to_string());
        }
        assert!(
            hit.len() >= DEFAULT_COLORS.len() / 2,
            "expected at least half the palette to appear in 200 samples, got {}",
            hit.len()
        );
    }
}
