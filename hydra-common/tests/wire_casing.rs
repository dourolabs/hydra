//! CI guard for the canonical kebab-case casing on unit-variant wire enums
//! under `hydra-common/src/`.
//!
//! Per `docs/architecture/api-wire-contract.md:54`, wire enums use
//! `#[serde(rename_all = "kebab-case")]`. This test walks every `.rs` file
//! under `src/`, flags any `#[serde(rename_all = "snake_case")]` attribute
//! that is not on a tagged-union (`tag = "..."`) enum, and requires either
//! the canonical `kebab-case` casing or a `// wire-casing-exempt: <reason>`
//! marker on the line immediately above the attribute.
//!
//! The marker is the escape hatch for the legitimate hold-outs enumerated in
//! the audit ([[i-lemrnuob]]): wire-breaking unit-variant enums whose
//! rename would change a published wire string (e.g. `SseEventType`,
//! `MergeBlockedCode`), and data-bearing enums that fall outside the
//! audit's scope but match the same grep pattern (e.g. `TaskError`).

use std::fs;
use std::path::{Path, PathBuf};

const SNAKE_CASE_ATTR: &str = "rename_all = \"snake_case\"";
const EXEMPT_MARKER: &str = "wire-casing-exempt:";

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read src/ dir") {
        let entry = entry.expect("read dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn check_file(path: &Path) -> Vec<String> {
    let content = fs::read_to_string(path).expect("read source file");
    let lines: Vec<&str> = content.lines().collect();
    let mut violations = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !line.contains(SNAKE_CASE_ATTR) {
            continue;
        }
        // Skip tagged-union enums — `tag = "..."` on the same `#[serde(...)]`
        // attribute means this enum is internally tagged and snake_case
        // applies to the discriminator string. Tagged enums are out of scope
        // for this guard.
        if line.contains("tag =") {
            continue;
        }
        // The marker must be on the line immediately above the attribute.
        // Locating it precisely (rather than scanning the whole attribute
        // block) keeps the spec unambiguous for future contributors.
        let has_marker = i
            .checked_sub(1)
            .map(|j| lines[j].contains(EXEMPT_MARKER))
            .unwrap_or(false);
        if !has_marker {
            violations.push(format!(
                "{}:{}: `#[serde({SNAKE_CASE_ATTR})]` on a unit-variant wire enum. \
                 Use `rename_all = \"kebab-case\"` (the canonical convention per \
                 `docs/architecture/api-wire-contract.md:54`), or add `// {EXEMPT_MARKER} <reason>` \
                 on the line immediately above the attribute.",
                path.display(),
                i + 1,
            ));
        }
    }
    violations
}

#[test]
fn unit_variant_wire_enums_use_kebab_case() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rs_files(&src, &mut files);
    let mut violations: Vec<String> = files.iter().flat_map(|p| check_file(p)).collect();
    violations.sort();
    assert!(
        violations.is_empty(),
        "wire-casing guard found violations:\n{}",
        violations.join("\n"),
    );
}
