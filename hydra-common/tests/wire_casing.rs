//! CI guard for the canonical kebab-case casing on unit-variant wire enums
//! under `hydra-common/src/`.
//!
//! Per `docs/architecture/api-wire-contract.md`, unit-variant wire enums use
//! `#[serde(rename_all = "kebab-case")]`. This test walks every `.rs` file
//! under `src/` and enforces two complementary rules:
//!
//! 1. **No `snake_case` on unit-variant enums.** Any non-tagged-union
//!    occurrence of `#[serde(rename_all = "snake_case")]` is flagged.
//! 2. **No PascalCase (serde default) on unit-variant Serialize enums.**
//!    Any `pub enum` in `src/` that derives `Serialize`/`Deserialize`, has
//!    only unit variants, and is missing a `rename_all` attribute is
//!    flagged. This closes the gap that let `PatchStatus` and
//!    `GithubCiState` ship as PascalCase wire types — the serde default
//!    PascalCase casing is no longer silently accepted.
//!
//! Both rules accept an escape hatch: a `// wire-casing-exempt: <reason>`
//! marker. For the snake_case rule the marker must be on the line
//! immediately above the attribute; for the PascalCase rule the marker may
//! appear anywhere in the attribute block above the `pub enum` declaration.
//!
//! The marker covers legitimate hold-outs enumerated in the audit
//! ([[i-lemrnuob]]): wire-breaking unit-variant enums whose rename would
//! change a published wire string (e.g. `SseEventType`, `MergeBlockedCode`),
//! and data-bearing enums that fall outside the audit's scope but match the
//! same grep pattern (e.g. `TaskError`).

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

fn check_snake_case(path: &Path) -> Vec<String> {
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
                 `docs/architecture/api-wire-contract.md`), or add `// {EXEMPT_MARKER} <reason>` \
                 on the line immediately above the attribute.",
                path.display(),
                i + 1,
            ));
        }
    }
    violations
}

fn check_pascal_case(label: &str, content: &str) -> Vec<String> {
    let lines: Vec<&str> = content.lines().collect();
    let mut violations = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        let Some(rest) = trimmed.strip_prefix("pub enum ") else {
            i += 1;
            continue;
        };
        let name_end = rest
            .find(|c: char| !c.is_alphanumeric() && c != '_')
            .unwrap_or(rest.len());
        let enum_name = &rest[..name_end];

        // Walk back through the attribute block: `#[...]` lines, `///` doc
        // comments, `//` comments, and blank lines. Stop at the first
        // unrelated line.
        let mut attr_text = String::new();
        let mut exempt = false;
        let mut j = i;
        while j > 0 {
            j -= 1;
            let s = lines[j].trim_start();
            if s.starts_with("#[") {
                attr_text.push_str(s);
                attr_text.push('\n');
            } else if s.starts_with("///") || s.starts_with("//!") {
                continue;
            } else if let Some(after_slashes) = s.strip_prefix("//") {
                if after_slashes.trim_start().starts_with(EXEMPT_MARKER) {
                    exempt = true;
                }
            } else if s.is_empty() {
                continue;
            } else {
                break;
            }
        }

        let derives_serde = attr_text.contains("Serialize") || attr_text.contains("Deserialize");
        let has_rename_all = attr_text.contains("rename_all");
        let has_tag = attr_text.contains("tag =") || attr_text.contains("tag=");

        let Some((body_start, body_end)) = find_body_span(&lines, i) else {
            i += 1;
            continue;
        };

        let is_unit = all_unit_variants(&lines, body_start, body_end);

        if derives_serde && is_unit && !has_rename_all && !has_tag && !exempt {
            violations.push(format!(
                "{}:{}: `pub enum {}` derives Serialize/Deserialize, has only unit \
                 variants, and is missing a `rename_all` attribute. The serde-default \
                 PascalCase casing is not a permitted wire format. Add \
                 `#[serde(rename_all = \"kebab-case\")]` (the canonical convention per \
                 `docs/architecture/api-wire-contract.md`), or add `// {} <reason>` in \
                 the attribute block above the declaration.",
                label,
                i + 1,
                enum_name,
                EXEMPT_MARKER,
            ));
        }

        i = body_end + 1;
    }
    violations
}

fn find_body_span(lines: &[&str], start: usize) -> Option<(usize, usize)> {
    let mut depth: i32 = 0;
    let mut body_start: Option<usize> = None;
    for (k, line) in lines.iter().enumerate().skip(start) {
        let active = strip_line_comment(line);
        let bytes = active.as_bytes();
        let mut in_str = false;
        let mut idx = 0;
        while idx < bytes.len() {
            let c = bytes[idx];
            if c == b'"' && (idx == 0 || bytes[idx - 1] != b'\\') {
                in_str = !in_str;
            } else if !in_str {
                match c {
                    b'{' => {
                        if depth == 0 {
                            body_start = Some(k);
                        }
                        depth += 1;
                    }
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            return body_start.map(|s| (s, k));
                        }
                    }
                    _ => {}
                }
            }
            idx += 1;
        }
    }
    None
}

fn strip_line_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_str = false;
    let mut idx = 0;
    while idx < bytes.len() {
        let c = bytes[idx];
        if c == b'"' && (idx == 0 || bytes[idx - 1] != b'\\') {
            in_str = !in_str;
        } else if !in_str && c == b'/' && idx + 1 < bytes.len() && bytes[idx + 1] == b'/' {
            return &line[..idx];
        }
        idx += 1;
    }
    line
}

fn all_unit_variants(lines: &[&str], body_start: usize, body_end: usize) -> bool {
    for (k, &line) in lines.iter().enumerate().take(body_end + 1).skip(body_start) {
        let mut s = line;
        if k == body_start {
            // Skip everything up to and including the opening `{`.
            if let Some(idx) = s.find('{') {
                s = &s[idx + 1..];
            }
        }
        let s = s.trim();
        if s.is_empty() || s == "{" || s == "}" {
            continue;
        }
        if s.starts_with('#') || s.starts_with("//") {
            continue;
        }
        let bytes = s.as_bytes();
        if !bytes[0].is_ascii_uppercase() {
            continue;
        }
        let mut end = 0;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
            end += 1;
        }
        let next = s[end..].trim_start();
        if next.starts_with('(') || next.starts_with('{') {
            return false;
        }
    }
    true
}

#[test]
fn unit_variant_wire_enums_use_kebab_case() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rs_files(&src, &mut files);
    let mut violations: Vec<String> = files.iter().flat_map(|p| check_snake_case(p)).collect();
    violations.sort();
    assert!(
        violations.is_empty(),
        "wire-casing guard found violations:\n{}",
        violations.join("\n"),
    );
}

#[test]
fn unit_variant_wire_enums_have_rename_all() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rs_files(&src, &mut files);
    let mut violations: Vec<String> = files
        .iter()
        .flat_map(|p| {
            let content = fs::read_to_string(p).expect("read source file");
            check_pascal_case(&p.display().to_string(), &content)
        })
        .collect();
    violations.sort();
    assert!(
        violations.is_empty(),
        "wire-casing PascalCase guard found violations:\n{}",
        violations.join("\n"),
    );
}

#[test]
fn pascal_case_check_flags_broken_inline_enum() {
    let broken = r#"
use serde::Serialize;

#[derive(Serialize)]
pub enum FakeStatus {
    Foo,
    Bar,
}
"#;
    let violations = check_pascal_case("synthetic", broken);
    assert_eq!(violations.len(), 1, "expected one violation, got {violations:?}");
    assert!(violations[0].contains("FakeStatus"), "{violations:?}");
}

#[test]
fn pascal_case_check_accepts_rename_all() {
    let ok = r#"
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FakeStatus {
    Foo,
    Bar,
}
"#;
    let violations = check_pascal_case("synthetic", ok);
    assert!(violations.is_empty(), "{violations:?}");
}

#[test]
fn pascal_case_check_skips_tagged_enum() {
    let ok = r#"
use serde::Serialize;

#[derive(Serialize)]
#[serde(tag = "type")]
pub enum FakeMessage {
    Foo,
    Bar,
}
"#;
    let violations = check_pascal_case("synthetic", ok);
    assert!(violations.is_empty(), "{violations:?}");
}

#[test]
fn pascal_case_check_accepts_exempt_marker() {
    let ok = r#"
use serde::Serialize;

// wire-casing-exempt: published wire string, breaking rename would be a wire break
#[derive(Serialize)]
pub enum FakeStatus {
    Foo,
    Bar,
}
"#;
    let violations = check_pascal_case("synthetic", ok);
    assert!(violations.is_empty(), "{violations:?}");
}

#[test]
fn pascal_case_check_skips_non_unit_variants() {
    let ok_tuple = r#"
use serde::Serialize;

#[derive(Serialize)]
pub enum FakePayload {
    Foo(String),
    Bar,
}
"#;
    let ok_struct = r#"
use serde::Serialize;

#[derive(Serialize)]
pub enum FakePayload {
    Foo { x: i32 },
    Bar,
}
"#;
    assert!(check_pascal_case("synthetic", ok_tuple).is_empty());
    assert!(check_pascal_case("synthetic", ok_struct).is_empty());
}

#[test]
fn pascal_case_check_skips_enums_without_serde_derive() {
    let ok = r#"
#[derive(Debug, Clone)]
pub enum FakeStatus {
    Foo,
    Bar,
}
"#;
    let violations = check_pascal_case("synthetic", ok);
    assert!(violations.is_empty(), "{violations:?}");
}
