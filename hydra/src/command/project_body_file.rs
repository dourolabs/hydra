//! Parser for the single-status body file passed to
//! `hydra projects status create` / `hydra projects status update`
//! via `--body-file`. Tries JSON first, then YAML.

use anyhow::{bail, Context, Result};
use hydra_common::api::v1::projects::StatusDefinition;
use std::path::Path;

pub fn load_status_body_file(path: &Path) -> Result<StatusDefinition> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read status body file '{}'", path.display()))?;
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        bail!("status body file '{}' is empty", path.display());
    }
    parse_status_body(trimmed)
        .with_context(|| format!("failed to parse status body file '{}'", path.display()))
}

pub fn parse_status_body(contents: &str) -> Result<StatusDefinition> {
    if let Ok(body) = serde_json::from_str::<StatusDefinition>(contents) {
        return Ok(body);
    }
    Ok(serde_yaml_ng::from_str::<StatusDefinition>(contents)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hydra_common::api::v1::projects::StatusKey;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_body(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file
    }

    #[test]
    fn load_status_body_file_parses_json() {
        let file = write_body(
            r##"{
                "key": "open",
                "label": "Open",
                "color": "#abcdef",
                "unblocks_parents": false,
                "unblocks_dependents": false,
                "cascades_to_children": false
            }"##,
        );
        let body = load_status_body_file(file.path()).unwrap();
        assert_eq!(body.key, StatusKey::try_new("open").unwrap());
    }

    #[test]
    fn load_status_body_file_parses_yaml() {
        let file = write_body(
            r##"
key: open
label: Open
color: "#abcdef"
unblocks_parents: false
unblocks_dependents: false
cascades_to_children: false
"##,
        );
        let body = load_status_body_file(file.path()).unwrap();
        assert_eq!(body.key, StatusKey::try_new("open").unwrap());
    }

    #[test]
    fn load_status_body_file_parses_yaml_with_principal_tag_form() {
        let file = write_body(
            r##"
key: backlog
label: Backlog
color: "#9b59b6"
unblocks_parents: false
unblocks_dependents: false
cascades_to_children: false
on_enter:
  assign_to: !Agent { name: pm }
"##,
        );
        let body = load_status_body_file(file.path()).unwrap();
        let on_enter = body.on_enter.as_ref().expect("on_enter present");
        let assignee = on_enter.assign_to.as_ref().expect("assign_to present");
        assert!(matches!(
            assignee,
            hydra_common::principal::Principal::Agent { name } if name.as_str() == "pm"
        ));
    }

    #[test]
    fn load_status_body_file_rejects_empty() {
        let file = write_body("");
        let err = load_status_body_file(file.path()).unwrap_err();
        assert!(err.to_string().contains("is empty"));
    }

    #[test]
    fn load_status_body_file_rejects_malformed() {
        let file = write_body("{not valid");
        let err = load_status_body_file(file.path()).unwrap_err();
        assert!(err.to_string().contains("failed to parse"), "got: {err:?}");
    }

    #[test]
    fn load_status_body_file_preserves_prompt_path() {
        let file = write_body(
            r##"
key: backlog
label: Backlog
color: "#9b59b6"
unblocks_parents: false
unblocks_dependents: false
cascades_to_children: false
prompt_path: /projects/engineering-v2/statuses/backlog.md
"##,
        );
        let body = load_status_body_file(file.path()).unwrap();
        assert_eq!(
            body.prompt_path.as_deref(),
            Some("/projects/engineering-v2/statuses/backlog.md"),
        );
    }

    #[test]
    fn load_status_body_file_preserves_auto_archive_after_seconds() {
        let file = write_body(
            r##"
key: done
label: Done
color: "#27ae60"
unblocks_parents: false
unblocks_dependents: false
cascades_to_children: false
auto_archive_after_seconds: 1209600
"##,
        );
        let body = load_status_body_file(file.path()).unwrap();
        assert_eq!(body.auto_archive_after_seconds, Some(1_209_600));
    }

    #[test]
    fn load_status_body_file_defaults_auto_archive_after_seconds_when_absent() {
        let file = write_body(
            r##"
key: done
label: Done
color: "#27ae60"
unblocks_parents: false
unblocks_dependents: false
cascades_to_children: false
"##,
        );
        let body = load_status_body_file(file.path()).unwrap();
        assert_eq!(body.auto_archive_after_seconds, None);
    }

    #[test]
    fn load_status_body_file_preserves_suppress_sessions() {
        let file = write_body(
            r##"
key: parked
label: Parked
color: "#95a5a6"
unblocks_parents: false
unblocks_dependents: false
cascades_to_children: false
suppress_sessions: true
"##,
        );
        let body = load_status_body_file(file.path()).unwrap();
        assert!(body.suppress_sessions);
    }

    #[test]
    fn load_status_body_file_defaults_suppress_sessions_when_absent() {
        let file = write_body(
            r##"
key: parked
label: Parked
color: "#95a5a6"
unblocks_parents: false
unblocks_dependents: false
cascades_to_children: false
"##,
        );
        let body = load_status_body_file(file.path()).unwrap();
        assert!(!body.suppress_sessions);
    }
}
