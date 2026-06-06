//! Parser for the project body file passed to `hydra projects create` /
//! `hydra projects update` via `--body-file`. Tries JSON first, then YAML.
//!
//! Split into its own module so the e2e fixture round-trip test in
//! `hydra-single-player` can drive the exact same parser the CLI uses.

use anyhow::{bail, Context, Result};
use hydra_common::api::v1::projects::StatusKey;
use std::path::Path;

/// Body file payload for `projects create` / `projects update`. Describes a
/// project's status list and its `default_status_key`. The CLI fills in the
/// `key`, `name`, and `creator` fields on top of this.
#[derive(Debug, serde::Deserialize)]
pub struct ProjectBodyFile {
    #[serde(default)]
    pub statuses: Vec<hydra_common::api::v1::projects::StatusDefinition>,
    pub default_status_key: StatusKey,
}

pub fn load_body_file(path: &Path) -> Result<ProjectBodyFile> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read project body file '{}'", path.display()))?;
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        bail!("project body file '{}' is empty", path.display());
    }
    if let Ok(body) = serde_json::from_str::<ProjectBodyFile>(trimmed) {
        return Ok(body);
    }
    serde_yaml_ng::from_str::<ProjectBodyFile>(trimmed)
        .with_context(|| format!("failed to parse project body file '{}'", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hydra_common::api::v1::projects::{IconKey, StatusDefinition};
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_body(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file
    }

    #[test]
    fn load_body_file_parses_json() {
        let file = write_body(
            r##"{
                "statuses": [
                    {
                        "key": "open",
                        "label": "Open",
                        "icon": "circle",
                        "color": "#abcdef",
                        "unblocks_parents": false,
                        "unblocks_dependents": false,
                        "cascades_to_children": false
                    }
                ],
                "default_status_key": "open"
            }"##,
        );
        let body = load_body_file(file.path()).unwrap();
        assert_eq!(body.statuses.len(), 1);
        assert_eq!(body.statuses[0].key, StatusKey::try_new("open").unwrap());
        assert_eq!(body.default_status_key, StatusKey::try_new("open").unwrap());
    }

    #[test]
    fn load_body_file_parses_yaml() {
        let file = write_body(
            r##"
statuses:
  - key: open
    label: Open
    icon: circle
    color: "#abcdef"
    unblocks_parents: false
    unblocks_dependents: false
    cascades_to_children: false
default_status_key: open
"##,
        );
        let body = load_body_file(file.path()).unwrap();
        assert_eq!(body.statuses.len(), 1);
        assert_eq!(body.default_status_key, StatusKey::try_new("open").unwrap());
    }

    #[test]
    fn load_body_file_parses_yaml_with_principal_tag_form() {
        let file = write_body(
            r##"
statuses:
  - key: backlog
    label: Backlog
    icon: list
    color: "#9b59b6"
    unblocks_parents: false
    unblocks_dependents: false
    cascades_to_children: false
    on_enter:
      assign_to: !Agent { name: pm }
default_status_key: backlog
"##,
        );
        let body = load_body_file(file.path()).unwrap();
        let on_enter = body.statuses[0]
            .on_enter
            .as_ref()
            .expect("on_enter present");
        let assignee = on_enter.assign_to.as_ref().expect("assign_to present");
        assert!(matches!(
            assignee,
            hydra_common::principal::Principal::Agent { name } if name.as_str() == "pm"
        ));
    }

    #[test]
    fn load_body_file_rejects_empty() {
        let file = write_body("");
        let err = load_body_file(file.path()).unwrap_err();
        assert!(err.to_string().contains("is empty"));
    }

    #[test]
    fn load_body_file_rejects_malformed() {
        let file = write_body("{not valid");
        let err = load_body_file(file.path()).unwrap_err();
        assert!(err.to_string().contains("failed to parse"), "got: {err:?}");
    }

    #[test]
    fn status_definition_roundtrips_through_body_file() {
        let def = StatusDefinition::new(
            StatusKey::try_new("inbox").unwrap(),
            "Inbox".into(),
            IconKey::try_new("inbox").unwrap(),
            "#ffaa00".parse().unwrap(),
            false,
            false,
            false,
            None,
        );
        let json = format!(
            r#"{{ "statuses": [{}], "default_status_key": "inbox" }}"#,
            serde_json::to_string(&def).unwrap()
        );
        let file = write_body(&json);
        let body = load_body_file(file.path()).unwrap();
        assert_eq!(body.statuses, vec![def]);
    }
}
