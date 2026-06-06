//! Round-trip every structured fixture under `tests/e2e/fixtures/` through
//! the same parser the `hydra-sp` binary uses, so a fixture that the runtime
//! parser rejects fails `cargo test` instead of slipping through to the e2e
//! tester as a downstream scenario skip.
//!
//! Today every YAML/JSON fixture under `tests/e2e/fixtures/` is a project
//! body file (consumed by `hydra projects create --body-file`). The walk is
//! deliberately broad (`**/*.yaml`, `**/*.yml`, `**/*.json`) so a newly
//! added fixture is covered automatically; if a future fixture targets a
//! different parser entry point, dispatch on path here.

use hydra::command::project_body_file::load_body_file;
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
        if let Err(err) = load_body_file(path) {
            failures.push(format!(
                "fixture {} failed to parse: {err:?}",
                path.display(),
            ));
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
