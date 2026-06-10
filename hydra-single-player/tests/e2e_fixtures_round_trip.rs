//! Round-trip every form fixture under `tests/e2e/fixtures/forms/` through
//! the same `Form` parser the `apply_status_on_enter` automation uses, so
//! a fixture that the runtime parser rejects fails `cargo test` instead of
//! slipping through to the e2e tester as a downstream scenario skip.

use hydra_common::api::v1::form::Form;
use std::path::{Path, PathBuf};

const STRUCTURED_EXTS: &[&str] = &["yaml", "yml", "json"];

#[test]
fn e2e_form_fixtures_round_trip_through_form_parser() {
    let forms_root = repo_root()
        .join("tests")
        .join("e2e")
        .join("fixtures")
        .join("forms");
    assert!(
        forms_root.is_dir(),
        "expected e2e form fixtures dir at {}",
        forms_root.display(),
    );

    let mut paths = Vec::new();
    collect_structured_fixtures(&forms_root, &mut paths);
    paths.sort();
    assert!(
        !paths.is_empty(),
        "no YAML/JSON fixtures found under {} -- has the layout moved?",
        forms_root.display(),
    );

    let mut failures = Vec::new();
    for path in &paths {
        if let Err(err) = parse_form_fixture(path) {
            failures.push(format!("fixture {} failed to parse: {err}", path.display()));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} form fixture(s) failed to parse:\n{}",
        failures.len(),
        paths.len(),
        failures.join("\n"),
    );
}

fn parse_form_fixture(path: &Path) -> Result<(), String> {
    let body = std::fs::read_to_string(path).map_err(|e| format!("read failed: {e}"))?;
    let form: Form = serde_yaml_ng::from_str(&body).map_err(|e| format!("not valid YAML: {e}"))?;
    form.validate_field_keys()
        .map_err(|e| format!("invalid fields: {e}"))?;
    Ok(())
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("hydra-single-player should sit under the workspace root")
        .to_path_buf()
}

fn collect_structured_fixtures(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("read_dir({}) failed: {err}", dir.display()));
    for entry in entries {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_structured_fixtures(&path, out);
        } else if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
            if STRUCTURED_EXTS.contains(&ext) {
                out.push(path);
            }
        }
    }
}
