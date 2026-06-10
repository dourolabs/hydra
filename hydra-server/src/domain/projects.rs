//! Default-project constants and seed for stores that lack a SQL
//! migration pipeline (e.g. `MemoryStore`). The five-status seed
//! reproduces the legacy hardcoded status semantics so issues created
//! before per-project statuses existed continue to resolve without
//! per-row migration.
//!
//! Flag table for the seeded statuses:
//!
//! | key           | unblocks_parents | unblocks_dependents | cascades_to_children | on_enter                                |
//! |---------------|------------------|---------------------|----------------------|-----------------------------------------|
//! | `open`        | false            | false               | false                | none                                    |
//! | `in-progress` | false            | false               | false                | none                                    |
//! | `closed`      | true             | true                | false                | clear_assignee=true, teardown_work=true |
//! | `dropped`     | true             | false               | true                 | clear_assignee=true, teardown_work=true |
//! | `failed`      | true             | false               | true                 | clear_assignee=true, teardown_work=true |
//!
//! Default status key is `open`. The terminal statuses
//! (`closed`/`dropped`/`failed`) carry `on_enter.clear_assignee = true`
//! so their `apply_status_on_enter` automation unsets the issue's
//! assignee, and `on_enter.teardown_work = true` so the
//! `teardown_issue_work` automation tears down any active sessions
//! attached to the issue.

use hydra_common::ProjectId;
use hydra_common::Rgb;
use hydra_common::api::v1::projects::{
    Project, ProjectKey, StatusDefinition, StatusKey, StatusOnEnter,
};
use hydra_common::api::v1::users::Username;

/// Wire string for the default project's slug. Stable: leaked to clients,
/// so don't rename without a migration plan.
pub const DEFAULT_PROJECT_KEY: &str = "default";

/// Stable, well-known `ProjectId` for the default project. Inserted by
/// the `seed_default_project` migration on SQL stores and seeded by
/// [`MemoryStore::new`] in-process. Must stay byte-for-byte identical to
/// the id in `sqlite-migrations/20260607000000_seed_default_project.sql`
/// (and the Postgres equivalent).
pub const DEFAULT_PROJECT_ID_STR: &str = "j-defaul";

/// Wire string for the synthesized [`no_project_sentinel`] project slug.
const NO_PROJECT_SENTINEL_KEY: &str = "no-project";

/// Wire string for the synthesized [`no_project_sentinel`] status slug.
const NO_PROJECT_SENTINEL_STATUS_KEY: &str = "none";

/// Username under which the default project is "owned". Stored verbatim
/// in the seed migration's `creator` column.
pub const SYSTEM_USERNAME: &str = "system";

/// Returns the stable [`ProjectId`] for the default project. Thin
/// wrapper over [`ProjectId::default_project`] so the literal lives in
/// one place ([`hydra_common::ids`]) and the server-side helper stays
/// available for existing call sites.
pub fn default_project_id() -> ProjectId {
    ProjectId::default_project()
}

/// Build the default-project [`Project`] value seeded by SQL migrations
/// and by [`MemoryStore::new`].
///
/// Status colors are explicit hex values approximating the existing
/// frontend badge palette. Any change here must be mirrored in the
/// `20260607000000_seed_default_project.sql` migrations (SQLite and
/// Postgres), or the SQL-backed and Memory-backed stores will disagree.
pub fn default_project_seed() -> Project {
    let mut open = StatusDefinition::new(
        status_key("open"),
        "Open".to_string(),
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
            rgb("#2ecc71"),
            true,
            true,
            false,
            Some(terminal_on_enter()),
        ),
        StatusDefinition::new(
            status_key("dropped"),
            "Dropped".to_string(),
            rgb("#795548"),
            true,
            false,
            true,
            Some(terminal_on_enter()),
        ),
        StatusDefinition::new(
            status_key("failed"),
            "Failed".to_string(),
            rgb("#e74c3c"),
            true,
            false,
            true,
            Some(terminal_on_enter()),
        ),
    ];

    let mut project = Project::new(
        ProjectKey::try_new(DEFAULT_PROJECT_KEY).expect("default project key is well-formed"),
        "Default".to_string(),
        statuses,
        Username::try_new(SYSTEM_USERNAME).expect("system username is well-formed"),
        false,
        1000.0,
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
        Username::try_new(SYSTEM_USERNAME).expect("system username is well-formed"),
        false,
        0.0,
    );
    (project, status)
}

fn status_key(value: &str) -> StatusKey {
    StatusKey::try_new(value).expect("default project status keys are well-formed")
}

fn rgb(value: &str) -> Rgb {
    value.parse().expect("default project colors are valid hex")
}

/// Build the `StatusOnEnter` shared by the default project's terminal
/// statuses (`closed`, `dropped`, `failed`): unset the issue's assignee
/// so the spawn dispatcher does not pick it back up, and kill any
/// `Created`/`Pending`/`Running` sessions still attached to the issue.
fn terminal_on_enter() -> StatusOnEnter {
    let mut on_enter = StatusOnEnter::new(None, None);
    on_enter.clear_assignee = true;
    on_enter.teardown_work = true;
    on_enter
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_project_id_is_well_formed() {
        let id = default_project_id();
        assert_eq!(id.as_ref(), DEFAULT_PROJECT_ID_STR);
    }

    #[test]
    fn default_project_seed_has_five_statuses() {
        assert_eq!(default_project_seed().statuses.len(), 5);
    }

    #[test]
    fn default_project_seed_has_unique_status_keys() {
        let seed = default_project_seed();
        let mut seen = std::collections::HashSet::new();
        for status in &seed.statuses {
            assert!(
                seen.insert(status.key.as_str().to_string()),
                "duplicate status key '{}' in default project seed",
                status.key
            );
        }
    }

    /// Every legacy status wire string must resolve to a status in the
    /// default project. This is the legacy-compat contract for issues
    /// that previously had no `project_id`.
    #[test]
    fn every_legacy_status_string_resolves() {
        let project = default_project_seed();
        for status_slug in ["open", "in-progress", "closed", "dropped", "failed"] {
            let key = StatusKey::try_new(status_slug).unwrap();
            assert!(
                project.find_status(&key).is_some(),
                "default project is missing status '{status_slug}'"
            );
        }
    }

    /// Lock the flag values for each default-project status. A change
    /// here is a behavior change for every default project — update with intent.
    #[test]
    fn default_project_seed_flags_match_design_table() {
        struct Expected {
            key: &'static str,
            unblocks_parents: bool,
            unblocks_dependents: bool,
            cascades_to_children: bool,
            // `None` ⇒ `on_enter` must be `None`. `Some((clear, kill))`
            // ⇒ `on_enter` must be `Some` with the given flag values
            // and `assign_to`/`attach_form` both `None`.
            on_enter: Option<(bool, bool)>,
        }
        let project = default_project_seed();
        let cases: &[Expected] = &[
            Expected {
                key: "open",
                unblocks_parents: false,
                unblocks_dependents: false,
                cascades_to_children: false,
                on_enter: None,
            },
            Expected {
                key: "in-progress",
                unblocks_parents: false,
                unblocks_dependents: false,
                cascades_to_children: false,
                on_enter: None,
            },
            Expected {
                key: "closed",
                unblocks_parents: true,
                unblocks_dependents: true,
                cascades_to_children: false,
                on_enter: Some((true, true)),
            },
            Expected {
                key: "dropped",
                unblocks_parents: true,
                unblocks_dependents: false,
                cascades_to_children: true,
                on_enter: Some((true, true)),
            },
            Expected {
                key: "failed",
                unblocks_parents: true,
                unblocks_dependents: false,
                cascades_to_children: true,
                on_enter: Some((true, true)),
            },
        ];
        for case in cases {
            let k = case.key;
            let key = StatusKey::try_new(k).unwrap();
            let def = project
                .find_status(&key)
                .unwrap_or_else(|| panic!("missing status {k}"));
            assert_eq!(
                def.unblocks_parents, case.unblocks_parents,
                "unblocks_parents for {k}"
            );
            assert_eq!(
                def.unblocks_dependents, case.unblocks_dependents,
                "unblocks_dependents for {k}"
            );
            assert_eq!(
                def.cascades_to_children, case.cascades_to_children,
                "cascades_to_children for {k}"
            );
            match case.on_enter {
                None => assert!(def.on_enter.is_none(), "on_enter must be None for {k}"),
                Some((want_clear, want_teardown)) => {
                    let on_enter = def
                        .on_enter
                        .as_ref()
                        .unwrap_or_else(|| panic!("on_enter must be Some for {k}"));
                    assert!(
                        on_enter.assign_to.is_none(),
                        "on_enter.assign_to must be None for {k}"
                    );
                    assert!(
                        on_enter.attach_form.is_none(),
                        "on_enter.attach_form must be None for {k}"
                    );
                    assert_eq!(
                        on_enter.clear_assignee, want_clear,
                        "on_enter.clear_assignee for {k}"
                    );
                    assert_eq!(
                        on_enter.teardown_work, want_teardown,
                        "on_enter.teardown_work for {k}"
                    );
                }
            }
        }
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
    fn default_project_seed_has_interactive_false_for_every_status() {
        for status in &default_project_seed().statuses {
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

    /// Default-project statuses must all leave `suppress_sessions` false —
    /// flipping any of them would silently change spawn behavior for the
    /// built-in statuses.
    #[test]
    fn default_project_seed_has_suppress_sessions_false_for_every_status() {
        for status in &default_project_seed().statuses {
            assert!(
                !status.suppress_sessions,
                "default project status '{}' must not suppress sessions",
                status.key
            );
        }
    }

    #[test]
    fn no_project_sentinel_status_does_not_suppress_sessions() {
        let (_, status) = no_project_sentinel();
        assert!(!status.suppress_sessions);
    }

    /// Locks the default-project priority to `1000.0`. The seed migration
    /// in `20260607000000_seed_default_project.sql` predates the priority
    /// column, so the value is supplied by the rank backfill in
    /// `20260610000000_add_projects_priority.sql`; the Rust seed and the
    /// migrated row must agree.
    #[test]
    fn default_project_seed_priority_is_one_thousand() {
        assert_eq!(default_project_seed().priority, 1000.0);
    }

    /// The no-project sentinel is synthetic and never listed, so its
    /// priority is irrelevant — but pin it to `0.0` so any drift is loud.
    #[test]
    fn no_project_sentinel_priority_is_zero() {
        let (project, _) = no_project_sentinel();
        assert_eq!(project.priority, 0.0);
    }

    /// Locks the per-layer `prompt_path` references for the default project.
    #[test]
    fn default_project_seed_sets_prompt_paths_for_non_terminal_statuses_only() {
        let project = default_project_seed();
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
