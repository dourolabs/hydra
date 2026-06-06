//! `DefaultProject`: the synthesized project used for issues with no
//! `project_id`. Reproduces today's `IssueStatus` semantics so legacy
//! issues continue to resolve without a per-row migration.
//!
//! Flag table for the synthesized statuses:
//!
//! | key           | unblocks_parents | unblocks_dependents | cascades_to_children |
//! |---------------|------------------|---------------------|----------------------|
//! | `open`        | false            | false               | false                |
//! | `in-progress` | false            | false               | false                |
//! | `closed`      | true             | true                | false                |
//! | `dropped`     | true             | false               | true                 |
//! | `failed`      | true             | false               | true                 |
//!
//! Default status key is `open`; no status has `on_enter` automation.

use hydra_common::Rgb;
use hydra_common::api::v1::projects::{IconKey, Project, ProjectKey, StatusDefinition, StatusKey};
use hydra_common::api::v1::users::Username;
use std::sync::OnceLock;

/// Wire string for the default project's slug. Stable: leaked to clients
/// once routes land (PR 3), so don't rename without a migration plan.
pub const DEFAULT_PROJECT_KEY: &str = "default";

/// Wire string for the synthesized [`no_project_sentinel`] project slug.
const NO_PROJECT_SENTINEL_KEY: &str = "no-project";

/// Wire string for the synthesized [`no_project_sentinel`] status slug.
const NO_PROJECT_SENTINEL_STATUS_KEY: &str = "none";

/// Username under which the default project is "owned". Synthesized,
/// never written to storage — used only to populate
/// [`Project::creator`] on the in-memory const.
const SYSTEM_USERNAME: &str = "system";

/// The synthesized default project, lazily constructed once per process.
///
/// The five statuses (`open`, `in-progress`, `closed`, `dropped`, `failed`)
/// reproduce today's `IssueStatus` flag semantics — see the table in this
/// module's top-level doc-comment. The status colors are explicit hex values
/// approximating the existing frontend badge palette
/// (`hydra-web/packages/ui/src/theme/tokens.css:78-83` —
/// `--s-open` blue, `--s-progress` amber, `--s-closed` green,
/// `--s-failed` red, `--s-dropped` dim red-brown) so badge appearance
/// is preserved when the frontend switches from the hardcoded
/// `statusMapping.ts` to `resolved_status.color` in PR 5.
pub fn default_project() -> &'static Project {
    static INSTANCE: OnceLock<Project> = OnceLock::new();
    INSTANCE.get_or_init(build_default_project)
}

fn build_default_project() -> Project {
    let mut open = StatusDefinition::new(
        status_key("open"),
        "Open".to_string(),
        icon_key("circle"),
        // Matches `--s-open` (blue) at tokens.css:78.
        rgb("#3498db"),
        false,
        false,
        false,
        None,
    );
    open.prompt_path = Some("/projects/default/statuses/open.md".to_string());

    let mut in_progress = StatusDefinition::new(
        status_key("in-progress"),
        "In progress".to_string(),
        icon_key("circle-dot"),
        // Matches `--s-progress` (amber) at tokens.css:79.
        rgb("#f1c40f"),
        false,
        false,
        false,
        None,
    );
    in_progress.prompt_path = Some("/projects/default/statuses/in-progress.md".to_string());

    let statuses = vec![
        open,
        in_progress,
        StatusDefinition::new(
            status_key("closed"),
            "Closed".to_string(),
            icon_key("check-circle"),
            // Matches `--s-closed` (green) at tokens.css:80.
            rgb("#2ecc71"),
            true,
            true,
            false,
            None,
        ),
        StatusDefinition::new(
            status_key("dropped"),
            "Dropped".to_string(),
            icon_key("x-circle"),
            // Matches `--s-dropped` (dim red-brown) at tokens.css:82.
            rgb("#795548"),
            true,
            false,
            true,
            None,
        ),
        StatusDefinition::new(
            status_key("failed"),
            "Failed".to_string(),
            icon_key("alert-circle"),
            // Matches `--s-failed` (red) at tokens.css:81.
            rgb("#e74c3c"),
            true,
            false,
            true,
            None,
        ),
    ];

    let mut project = Project::new(
        ProjectKey::try_new(DEFAULT_PROJECT_KEY).expect("default project key is well-formed"),
        "Default".to_string(),
        statuses,
        status_key("open"),
        Username::try_new(SYSTEM_USERNAME).expect("system username is well-formed"),
        false,
    );
    project.prompt_path = Some("/projects/default/prompt.md".to_string());
    project
}

/// Returns a `(Project, StatusDefinition)` pair with no `prompt_path` on
/// either side, used for sessions that are not associated with any issue
/// (e.g. conversation sessions). The four-level prompt resolver in PR 1
/// sees `None` paths on both layers and emits empty slices for the
/// project and status layers, so the spawned `system_prompt` is
/// system + agent only.
pub fn no_project_sentinel() -> (Project, StatusDefinition) {
    let status = StatusDefinition::new(
        status_key(NO_PROJECT_SENTINEL_STATUS_KEY),
        "None".to_string(),
        icon_key("circle"),
        rgb("#000000"),
        false,
        false,
        false,
        None,
    );
    let project = Project::new(
        ProjectKey::try_new(NO_PROJECT_SENTINEL_KEY)
            .expect("no-project sentinel key is well-formed"),
        "No project".to_string(),
        vec![status.clone()],
        status_key(NO_PROJECT_SENTINEL_STATUS_KEY),
        Username::try_new(SYSTEM_USERNAME).expect("system username is well-formed"),
        false,
    );
    (project, status)
}

fn status_key(value: &str) -> StatusKey {
    StatusKey::try_new(value).expect("default project status keys are well-formed")
}

fn icon_key(value: &str) -> IconKey {
    IconKey::try_new(value).expect("default project icon keys are well-formed")
}

fn rgb(value: &str) -> Rgb {
    value.parse().expect("default project colors are valid hex")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::issues::IssueStatus;

    #[test]
    fn default_project_validates() {
        default_project()
            .validate()
            .expect("default project must validate");
    }

    #[test]
    fn default_project_has_five_statuses() {
        assert_eq!(default_project().statuses.len(), 5);
    }

    #[test]
    fn default_project_default_status_is_open() {
        assert_eq!(default_project().default_status_key.as_str(), "open");
    }

    /// Every wire string produced by today's `IssueStatus` must resolve
    /// to a status in the default project. This is the legacy-compat
    /// contract for issues with no `project_id`.
    #[test]
    fn every_legacy_status_string_resolves() {
        for status in [
            IssueStatus::Open,
            IssueStatus::InProgress,
            IssueStatus::Closed,
            IssueStatus::Dropped,
            IssueStatus::Failed,
        ] {
            let key = StatusKey::try_new(status.as_str()).unwrap();
            assert!(
                default_project().find_status(&key).is_some(),
                "default project is missing status '{}'",
                status.as_str()
            );
        }
    }

    /// Lock the flag values for each default-project status. A change
    /// here is a behavior change for every default project — update with intent.
    #[test]
    fn default_project_flags_match_design_table() {
        let cases: &[(&str, bool, bool, bool)] = &[
            // (key, unblocks_parents, unblocks_dependents, cascades_to_children)
            ("open", false, false, false),
            ("in-progress", false, false, false),
            ("closed", true, true, false),
            ("dropped", true, false, true),
            ("failed", true, false, true),
        ];
        for (k, ub_p, ub_d, casc) in cases {
            let key = StatusKey::try_new(*k).unwrap();
            let def = default_project()
                .find_status(&key)
                .unwrap_or_else(|| panic!("missing status {k}"));
            assert_eq!(def.unblocks_parents, *ub_p, "unblocks_parents for {k}");
            assert_eq!(
                def.unblocks_dependents, *ub_d,
                "unblocks_dependents for {k}"
            );
            assert_eq!(
                def.cascades_to_children, *casc,
                "cascades_to_children for {k}"
            );
            assert!(def.on_enter.is_none(), "on_enter must be None for {k}");
        }
    }

    #[test]
    fn default_project_returns_same_instance() {
        let a = default_project() as *const Project;
        let b = default_project() as *const Project;
        assert_eq!(a, b);
    }

    #[test]
    fn no_project_sentinel_has_no_prompt_paths() {
        let (project, status) = no_project_sentinel();
        assert!(project.prompt_path.is_none());
        assert!(status.prompt_path.is_none());
    }

    /// The default project's statuses must all leave `interactive` false —
    /// flipping any of them would silently change spawn behavior for the
    /// built-in statuses.
    #[test]
    fn default_project_has_interactive_false_for_every_status() {
        for status in &default_project().statuses {
            assert!(
                !status.interactive,
                "default project status '{}' must not be interactive",
                status.key
            );
        }
    }

    #[test]
    fn no_project_sentinel_status_is_not_interactive() {
        let (_, status) = no_project_sentinel();
        assert!(!status.interactive);
    }

    /// Locks the per-layer `prompt_path` references for the default project
    /// shipped by PR 1 of [[d-rzreslz]]. The docs at these paths don't yet
    /// exist (PR 2 authors them); the resolver tolerates the gap and
    /// produces empty slices for the missing layers.
    #[test]
    fn default_project_sets_prompt_paths_for_non_terminal_statuses_only() {
        let project = default_project();
        assert_eq!(
            project.prompt_path.as_deref(),
            Some("/projects/default/prompt.md")
        );

        let expected: &[(&str, Option<&str>)] = &[
            ("open", Some("/projects/default/statuses/open.md")),
            (
                "in-progress",
                Some("/projects/default/statuses/in-progress.md"),
            ),
            ("closed", None),
            ("dropped", None),
            ("failed", None),
        ];
        for (key, want) in expected {
            let status_key = StatusKey::try_new(*key).unwrap();
            let def = project
                .find_status(&status_key)
                .unwrap_or_else(|| panic!("missing status {key}"));
            assert_eq!(
                def.prompt_path.as_deref(),
                *want,
                "prompt_path for status {key}"
            );
        }
    }
}
