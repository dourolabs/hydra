//! Round-trip every structured fixture under `tests/e2e/fixtures/` through
//! the same parser the `hydra-sp` binary uses, so a fixture that the runtime
//! parser rejects fails `cargo test` instead of slipping through to the e2e
//! tester as a downstream scenario skip.
//!
//! Fixture kinds are dispatched by path:
//! * `fixtures/forms/*.yaml` → parsed as `hydra_common::api::v1::form::Form`
//!   (the same wire format the `apply_status_on_enter` automation loads
//!   from the document store).
//! * everything else → parsed as a project body file (consumed by
//!   `hydra projects create --body-file`).
//!
//! The walk is deliberately broad (`**/*.yaml`, `**/*.yml`, `**/*.json`)
//! so a newly added fixture is covered automatically; if a future fixture
//! targets yet another parser entry point, extend the dispatch below.

use hydra::command::project_body_file::load_status_body_file;
use hydra_common::api::v1::form::Form;
use std::path::{Path, PathBuf};

const STRUCTURED_EXTS: &[&str] = &["yaml", "yml", "json"];

#[test]
fn e2e_fixtures_round_trip_through_body_file_parser() {
    let fixtures_root = repo_root().join("tests").join("e2e").join("fixtures");
    assert!(
        fixtures_root.is_dir(),
        "expected e2e fixtures dir at {}",
        fixtures_root.display(),
    );

    let mut paths = Vec::new();
    collect_structured_fixtures(&fixtures_root, &mut paths);
    paths.sort();
    assert!(
        !paths.is_empty(),
        "no YAML/JSON fixtures found under {} -- has the layout moved?",
        fixtures_root.display(),
    );

    let mut failures = Vec::new();
    for path in &paths {
        let result = if is_form_fixture(path, &fixtures_root) {
            parse_form_fixture(path)
        } else {
            load_status_body_file(path)
                .map(|_| ())
                .map_err(|e| format!("{e:?}"))
        };
        if let Err(err) = result {
            failures.push(format!("fixture {} failed to parse: {err}", path.display()));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} fixture file(s) failed to parse:\n{}",
        failures.len(),
        paths.len(),
        failures.join("\n"),
    );
}

/// Fixtures under `tests/e2e/fixtures/forms/` mirror documents pushed to
/// the `/forms/*` paths in the store at runtime — parse them with the
/// same `Form` schema that `apply_status_on_enter` uses.
fn is_form_fixture(path: &Path, fixtures_root: &Path) -> bool {
    path.strip_prefix(fixtures_root)
        .map(|rel| rel.starts_with("forms"))
        .unwrap_or(false)
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
