//! Phase 3 PR-1 bootstrap migration: synthesise a [`MergePolicy`] from each
//! repo's legacy [`RepoWorkflowConfig`], and close any lingering review- or
//! merge-request issues whose parent patch already reached a terminal status.
//!
//! See `/designs/merge-time-constraints.md` §4.6 and §6.6.
//!
//! The pass is idempotent: repos that already have `merge_policy.is_some()`
//! and issues already in a terminal status are skipped on subsequent runs.

use anyhow::Context;
use hydra_common::api::v1::patches::{PatchStatus, SearchPatchesQuery};
use hydra_common::api::v1::repositories::{
    DynamicRef, MergePolicy, MergerRule, Principal, RepoWorkflowConfig, ReviewerGroup,
    SearchRepositoriesQuery,
};
use hydra_common::api::v1::users::Username;
use thiserror::Error;

use crate::domain::actors::ActorRef;
use crate::domain::issues::{IssueStatus, IssueType};
use crate::domain::patches::PatchStatus as DomainPatchStatus;
use crate::store::Store;

/// Worker name used as the actor for writes performed by this migration.
pub const MIGRATION_WORKER_NAME: &str = "migration:synthesise-merge-policy";

/// Failure modes returned by [`synthesise`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SynthError {
    /// An `assignee` string starts with `$` but does not match the only
    /// supported legacy token (`$patch_creator`).
    #[error("unknown template token {token:?} in `{context}`; only `$patch_creator` is supported")]
    UnknownTemplateToken { token: String, context: String },

    /// An `assignee` field was empty / whitespace-only.
    #[error("empty assignee in `{context}`")]
    EmptyAssignee { context: String },
}

/// Translate a single legacy `assignee` template string into a [`Principal`].
///
/// The only template token retained by the synthesiser is `$patch_creator`
/// (translated to [`DynamicRef::PatchAuthor`]). Any other `$…` token is
/// rejected — the legacy automation supported substring substitution, but the
/// production data set only ever used the bare-token form, so refusing the
/// general case here keeps the synthesised policy auditable.
pub fn translate_principal(raw: &str, context: &str) -> Result<Principal, SynthError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(SynthError::EmptyAssignee {
            context: context.to_string(),
        });
    }

    if trimmed == "$patch_creator" {
        return Ok(Principal::Dynamic(DynamicRef::PatchAuthor));
    }

    // Anything else containing `$` is treated as an unsupported template
    // token. The legacy automation allowed substring substitution
    // (e.g. `prefix-$patch_creator`), but production policies never used
    // that form, and emitting a static-looking principal that secretly
    // contained `$` would be confusing once `patch_workflow` is removed.
    if trimmed.contains('$') {
        return Err(SynthError::UnknownTemplateToken {
            token: trimmed.to_string(),
            context: context.to_string(),
        });
    }

    Ok(Principal::User(Username::from(trimmed)))
}

/// Pure mapping from a legacy [`RepoWorkflowConfig`] to the synthesised
/// [`MergePolicy`]. See §4.6 of the design for the table this implements.
pub fn synthesise(workflow: &RepoWorkflowConfig) -> Result<MergePolicy, SynthError> {
    let mut reviewers = Vec::with_capacity(workflow.review_requests.len());
    for (idx, rr) in workflow.review_requests.iter().enumerate() {
        let context = format!("review_requests[{idx}].assignee");
        let principal = translate_principal(&rr.assignee, &context)?;
        reviewers.push(ReviewerGroup {
            label: None,
            any_of: vec![principal],
            count: 1,
            exclude_author: true,
        });
    }

    let mergers = match workflow.merge_request.as_ref() {
        Some(mr) => match mr.assignee.as_deref() {
            Some(assignee) => {
                let principal = translate_principal(assignee, "merge_request.assignee")?;
                Some(MergerRule {
                    any_of: vec![principal],
                })
            }
            // `merge_request: {}` (no assignee) reproduces the legacy
            // behaviour where any actor could file the MergeRequest issue —
            // i.e. no `mergers` restriction.
            None => None,
        },
        None => None,
    };

    Ok(MergePolicy { reviewers, mergers })
}

/// Summary of what the migration did on a single invocation.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MigrationReport {
    /// Number of repos that already had `merge_policy.is_some()` and were
    /// therefore left untouched.
    pub repos_already_migrated: usize,
    /// Number of repos that had `patch_workflow.is_some()` and `merge_policy
    /// .is_none()` and were updated by this run.
    pub repos_migrated: usize,
    /// Number of repos whose `patch_workflow` could not be synthesised (e.g.
    /// unknown template token). Such repos are logged and skipped — manual
    /// repair is required before the next phase.
    pub repos_with_synth_errors: usize,
    /// Number of MergeRequest issues closed (`PatchStatus::Merged`) or failed
    /// (`PatchStatus::Closed`) during the close-up pass.
    pub mr_issues_closed_or_failed: usize,
    /// Number of ReviewRequest issues dropped during the close-up pass.
    pub rr_issues_dropped: usize,
    /// Number of issues already in a terminal status and therefore skipped.
    pub issues_already_terminal: usize,
}

/// Run the one-shot bootstrap migration.
///
/// Iterates every repository and every patch in the store. Repositories that
/// already have a `merge_policy` are skipped. Patches that have not reached a
/// terminal status are skipped. Errors on individual records are logged and
/// counted so a single bad row does not abort the whole pass.
pub async fn run(store: &dyn Store) -> anyhow::Result<MigrationReport> {
    let mut report = MigrationReport::default();
    let actor = ActorRef::System {
        worker_name: MIGRATION_WORKER_NAME.into(),
        on_behalf_of: None,
    };

    synthesise_pass(store, &actor, &mut report)
        .await
        .context("synthesise merge_policy pass failed")?;
    close_lingering_pass(store, &actor, &mut report)
        .await
        .context("close lingering review/merge-request issues pass failed")?;

    tracing::info!(
        repos_migrated = report.repos_migrated,
        repos_already_migrated = report.repos_already_migrated,
        repos_with_synth_errors = report.repos_with_synth_errors,
        mr_issues_closed_or_failed = report.mr_issues_closed_or_failed,
        rr_issues_dropped = report.rr_issues_dropped,
        issues_already_terminal = report.issues_already_terminal,
        "synthesise_merge_policy migration complete"
    );

    Ok(report)
}

async fn synthesise_pass(
    store: &dyn Store,
    actor: &ActorRef,
    report: &mut MigrationReport,
) -> anyhow::Result<()> {
    // Exclude soft-deleted repos: writing a synthesised `merge_policy` to a
    // tombstone is harmless but pointless, and bloats the migration report.
    // Matches the convention used by `background::cleanup_branches`.
    let query = SearchRepositoriesQuery::new(Some(false));
    let repos = store
        .list_repositories(&query)
        .await
        .context("failed to list repositories")?;

    for (name, versioned) in repos {
        let repo = versioned.item;
        if repo.merge_policy.is_some() {
            report.repos_already_migrated += 1;
            continue;
        }
        let Some(workflow) = repo.patch_workflow.as_ref() else {
            // Repos without either field are out-of-scope for this pass and
            // already match the new "no policy = no restriction" default.
            continue;
        };

        let synthesised = match synthesise(workflow) {
            Ok(policy) => policy,
            Err(err) => {
                report.repos_with_synth_errors += 1;
                tracing::error!(
                    repo = %name,
                    error = %err,
                    "failed to synthesise merge_policy from patch_workflow; leaving repo unmigrated"
                );
                continue;
            }
        };

        // Start from the stored row so we preserve any fields not enumerated
        // here (the API type is `non_exhaustive`).
        let mut updated = repo.clone();
        updated.merge_policy = Some(synthesised);

        match store.update_repository(name.clone(), updated, actor).await {
            Ok(()) => {
                report.repos_migrated += 1;
                tracing::info!(
                    repo = %name,
                    "synthesised merge_policy from legacy patch_workflow"
                );
            }
            Err(err) => {
                report.repos_with_synth_errors += 1;
                tracing::error!(
                    repo = %name,
                    error = %err,
                    "failed to persist synthesised merge_policy"
                );
            }
        }
    }

    Ok(())
}

async fn close_lingering_pass(
    store: &dyn Store,
    actor: &ActorRef,
    report: &mut MigrationReport,
) -> anyhow::Result<()> {
    let query = SearchPatchesQuery::new(
        None,
        None,
        vec![PatchStatus::Merged, PatchStatus::Closed],
        None,
    );
    let patches = store
        .list_patches(&query)
        .await
        .context("failed to list terminal patches")?;

    for (patch_id, versioned) in patches {
        let status = versioned.item.status;
        // The store always speaks the domain type here; map to whether the
        // patch is in the merged-vs-other-terminal branch.
        let merged = matches!(status, DomainPatchStatus::Merged);

        let issue_ids = match store.get_issues_for_patch(&patch_id).await {
            Ok(ids) => ids,
            Err(err) => {
                tracing::error!(
                    patch_id = %patch_id,
                    error = %err,
                    "failed to load issues for terminal patch; skipping close-up"
                );
                continue;
            }
        };

        for issue_id in issue_ids {
            let issue = match store.get_issue(&issue_id, false).await {
                Ok(i) => i.item,
                Err(err) => {
                    tracing::warn!(
                        issue_id = %issue_id,
                        error = %err,
                        "failed to fetch issue while closing lingering RR/MR; skipping"
                    );
                    continue;
                }
            };

            if matches!(
                issue.status,
                IssueStatus::Closed | IssueStatus::Dropped | IssueStatus::Failed
            ) {
                report.issues_already_terminal += 1;
                continue;
            }

            let mut updated = issue.clone();
            match issue.issue_type {
                IssueType::MergeRequest => {
                    updated.status = if merged {
                        IssueStatus::Closed
                    } else {
                        IssueStatus::Failed
                    };
                }
                IssueType::ReviewRequest => {
                    updated.status = IssueStatus::Dropped;
                }
                _ => continue,
            }

            match store.update_issue(&issue_id, updated, actor).await {
                Ok(_) => match issue.issue_type {
                    IssueType::MergeRequest => {
                        report.mr_issues_closed_or_failed += 1;
                        tracing::info!(
                            issue_id = %issue_id,
                            patch_id = %patch_id,
                            patch_merged = merged,
                            "closed lingering merge-request issue"
                        );
                    }
                    IssueType::ReviewRequest => {
                        report.rr_issues_dropped += 1;
                        tracing::info!(
                            issue_id = %issue_id,
                            patch_id = %patch_id,
                            "dropped lingering review-request issue"
                        );
                    }
                    _ => {}
                },
                Err(err) => {
                    tracing::error!(
                        issue_id = %issue_id,
                        patch_id = %patch_id,
                        error = %err,
                        "failed to close lingering RR/MR issue"
                    );
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hydra_common::api::v1::repositories::{MergeRequestConfig, ReviewRequestConfig};

    fn user(name: &str) -> Principal {
        Principal::User(Username::from(name))
    }

    fn patch_author() -> Principal {
        Principal::Dynamic(DynamicRef::PatchAuthor)
    }

    fn reviewer_group(p: Principal) -> ReviewerGroup {
        ReviewerGroup {
            label: None,
            any_of: vec![p],
            count: 1,
            exclude_author: true,
        }
    }

    #[test]
    fn empty_workflow_synthesises_empty_policy() {
        let workflow = RepoWorkflowConfig::default();
        let policy = synthesise(&workflow).expect("empty workflow is valid");
        assert!(policy.reviewers.is_empty());
        assert!(policy.mergers.is_none());
    }

    #[test]
    fn reviewer_only_static_principal() {
        let workflow = RepoWorkflowConfig {
            review_requests: vec![ReviewRequestConfig {
                assignee: "alice".to_string(),
            }],
            merge_request: None,
        };
        let policy = synthesise(&workflow).expect("static reviewer is valid");
        assert_eq!(policy.reviewers, vec![reviewer_group(user("alice"))]);
        assert!(policy.mergers.is_none());
    }

    #[test]
    fn multiple_reviewers_each_become_their_own_group() {
        let workflow = RepoWorkflowConfig {
            review_requests: vec![
                ReviewRequestConfig {
                    assignee: "reviewer".to_string(),
                },
                ReviewRequestConfig {
                    assignee: "alice".to_string(),
                },
            ],
            merge_request: None,
        };
        let policy = synthesise(&workflow).expect("multiple reviewers are valid");
        assert_eq!(
            policy.reviewers,
            vec![
                reviewer_group(user("reviewer")),
                reviewer_group(user("alice")),
            ]
        );
        assert!(policy.mergers.is_none());
    }

    #[test]
    fn patch_creator_token_translates_to_dynamic_principal() {
        let workflow = RepoWorkflowConfig {
            review_requests: vec![ReviewRequestConfig {
                assignee: "$patch_creator".to_string(),
            }],
            merge_request: Some(MergeRequestConfig {
                assignee: Some("$patch_creator".to_string()),
            }),
        };
        let policy = synthesise(&workflow).expect("$patch_creator is valid");
        assert_eq!(policy.reviewers, vec![reviewer_group(patch_author())]);
        assert_eq!(
            policy.mergers,
            Some(MergerRule {
                any_of: vec![patch_author()]
            })
        );
    }

    #[test]
    fn merge_request_set_produces_mergers() {
        let workflow = RepoWorkflowConfig {
            review_requests: vec![],
            merge_request: Some(MergeRequestConfig {
                assignee: Some("alice".to_string()),
            }),
        };
        let policy = synthesise(&workflow).expect("static merger is valid");
        assert!(policy.reviewers.is_empty());
        assert_eq!(
            policy.mergers,
            Some(MergerRule {
                any_of: vec![user("alice")]
            })
        );
    }

    #[test]
    fn merge_request_absent_yields_no_mergers() {
        let workflow = RepoWorkflowConfig {
            review_requests: vec![ReviewRequestConfig {
                assignee: "alice".to_string(),
            }],
            merge_request: None,
        };
        let policy = synthesise(&workflow).expect("absent merge_request is valid");
        assert!(policy.mergers.is_none());
    }

    #[test]
    fn merge_request_present_without_assignee_yields_no_mergers() {
        let workflow = RepoWorkflowConfig {
            review_requests: vec![],
            merge_request: Some(MergeRequestConfig { assignee: None }),
        };
        let policy = synthesise(&workflow).expect("empty merge_request is valid");
        assert!(policy.mergers.is_none());
    }

    #[test]
    fn unknown_template_token_is_rejected() {
        let workflow = RepoWorkflowConfig {
            review_requests: vec![ReviewRequestConfig {
                assignee: "$parent_issue_creator".to_string(),
            }],
            merge_request: None,
        };
        let err = synthesise(&workflow).unwrap_err();
        assert!(
            matches!(
                err,
                SynthError::UnknownTemplateToken { ref token, .. }
                    if token == "$parent_issue_creator"
            ),
            "expected UnknownTemplateToken, got {err:?}",
        );
    }

    #[test]
    fn partial_substitution_form_is_rejected() {
        // Legacy substring-substitution form was never used in production
        // policies; refuse it so the synthesised policy is auditable.
        let workflow = RepoWorkflowConfig {
            review_requests: vec![ReviewRequestConfig {
                assignee: "prefix-$patch_creator".to_string(),
            }],
            merge_request: None,
        };
        let err = synthesise(&workflow).unwrap_err();
        assert!(
            matches!(err, SynthError::UnknownTemplateToken { .. }),
            "expected UnknownTemplateToken, got {err:?}",
        );
    }

    #[test]
    fn empty_assignee_is_rejected() {
        let workflow = RepoWorkflowConfig {
            review_requests: vec![ReviewRequestConfig {
                assignee: "   ".to_string(),
            }],
            merge_request: None,
        };
        let err = synthesise(&workflow).unwrap_err();
        assert!(
            matches!(err, SynthError::EmptyAssignee { .. }),
            "expected EmptyAssignee, got {err:?}",
        );
    }

    #[test]
    fn design_example_round_trips() {
        // The exact example from §4.6.
        let workflow = RepoWorkflowConfig {
            review_requests: vec![
                ReviewRequestConfig {
                    assignee: "reviewer".to_string(),
                },
                ReviewRequestConfig {
                    assignee: "alice".to_string(),
                },
            ],
            merge_request: Some(MergeRequestConfig {
                assignee: Some("$patch_creator".to_string()),
            }),
        };
        let policy = synthesise(&workflow).expect("design example is valid");
        assert_eq!(
            policy,
            MergePolicy {
                reviewers: vec![
                    reviewer_group(user("reviewer")),
                    reviewer_group(user("alice")),
                ],
                mergers: Some(MergerRule {
                    any_of: vec![patch_author()],
                }),
            }
        );
    }

    // ---- driver / store-level tests --------------------------------------

    use crate::domain::issues::{Issue, IssueDependency, IssueDependencyType};
    use crate::domain::patches::Patch;
    use crate::domain::users::Username as DomainUsername;
    use crate::store::{MemoryStore, ReadOnlyStore};
    use hydra_common::{RepoName, Repository};

    fn make_repo_with_workflow(workflow: RepoWorkflowConfig) -> Repository {
        Repository::new(
            "https://example.com/owner/repo.git".to_string(),
            Some("main".to_string()),
            None,
            Some(workflow),
        )
    }

    fn make_patch(status: DomainPatchStatus) -> Patch {
        Patch::new(
            "test patch".to_string(),
            "desc".to_string(),
            String::new(),
            status,
            false,
            None,
            DomainUsername::from("author"),
            Vec::new(),
            RepoName::new("acme", "service").unwrap(),
            None,
            None,
            None,
            None,
        )
    }

    fn make_review_request_issue(
        patch_id: &hydra_common::PatchId,
        parent_id: &hydra_common::IssueId,
        status: IssueStatus,
    ) -> Issue {
        Issue::new(
            IssueType::ReviewRequest,
            "Review Request".to_string(),
            "rr".to_string(),
            DomainUsername::from("system"),
            String::new(),
            status,
            Some("reviewer".to_string()),
            None,
            Vec::new(),
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
            vec![patch_id.clone()],
            None,
            None,
            None,
        )
    }

    fn make_merge_request_issue(
        patch_id: &hydra_common::PatchId,
        parent_id: &hydra_common::IssueId,
        status: IssueStatus,
    ) -> Issue {
        Issue::new(
            IssueType::MergeRequest,
            "Merge Request".to_string(),
            "mr".to_string(),
            DomainUsername::from("system"),
            String::new(),
            status,
            Some("merger".to_string()),
            None,
            Vec::new(),
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
            vec![patch_id.clone()],
            None,
            None,
            None,
        )
    }

    fn make_parent_task(patch_id: &hydra_common::PatchId) -> Issue {
        Issue::new(
            IssueType::Task,
            "Parent Task".to_string(),
            "parent".to_string(),
            DomainUsername::from("system"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            vec![patch_id.clone()],
            None,
            None,
            None,
        )
    }

    #[tokio::test]
    async fn run_migrates_repo_and_is_idempotent() {
        let store = MemoryStore::new();
        let actor = ActorRef::test();
        let repo_name = RepoName::new("acme", "service").unwrap();
        let workflow = RepoWorkflowConfig {
            review_requests: vec![ReviewRequestConfig {
                assignee: "reviewer".to_string(),
            }],
            merge_request: Some(MergeRequestConfig {
                assignee: Some("$patch_creator".to_string()),
            }),
        };
        store
            .add_repository(repo_name.clone(), make_repo_with_workflow(workflow), &actor)
            .await
            .unwrap();

        let report = run(&store).await.unwrap();
        assert_eq!(report.repos_migrated, 1);
        assert_eq!(report.repos_already_migrated, 0);
        assert_eq!(report.repos_with_synth_errors, 0);

        let after = store.get_repository(&repo_name, false).await.unwrap().item;
        let policy = after.merge_policy.expect("policy should be synthesised");
        assert_eq!(
            policy,
            MergePolicy {
                reviewers: vec![reviewer_group(user("reviewer"))],
                mergers: Some(MergerRule {
                    any_of: vec![patch_author()]
                }),
            }
        );
        // patch_workflow is preserved this PR (PR-2 drops it).
        assert!(after.patch_workflow.is_some());

        // Re-running is a no-op.
        let report2 = run(&store).await.unwrap();
        assert_eq!(report2.repos_migrated, 0);
        assert_eq!(report2.repos_already_migrated, 1);
    }

    #[tokio::test]
    async fn run_skips_repos_with_unknown_template_tokens() {
        let store = MemoryStore::new();
        let actor = ActorRef::test();
        let repo_name = RepoName::new("acme", "service").unwrap();
        let bad_workflow = RepoWorkflowConfig {
            review_requests: vec![ReviewRequestConfig {
                assignee: "$something_else".to_string(),
            }],
            merge_request: None,
        };
        store
            .add_repository(
                repo_name.clone(),
                make_repo_with_workflow(bad_workflow),
                &actor,
            )
            .await
            .unwrap();

        let report = run(&store).await.unwrap();
        assert_eq!(report.repos_migrated, 0);
        assert_eq!(report.repos_with_synth_errors, 1);

        let after = store.get_repository(&repo_name, false).await.unwrap().item;
        assert!(after.merge_policy.is_none());
    }

    #[tokio::test]
    async fn run_skips_soft_deleted_repos() {
        let store = MemoryStore::new();
        let actor = ActorRef::test();
        let repo_name = RepoName::new("acme", "service").unwrap();
        let workflow = RepoWorkflowConfig {
            review_requests: vec![ReviewRequestConfig {
                assignee: "reviewer".to_string(),
            }],
            merge_request: None,
        };
        store
            .add_repository(repo_name.clone(), make_repo_with_workflow(workflow), &actor)
            .await
            .unwrap();
        store.delete_repository(&repo_name, &actor).await.unwrap();

        let report = run(&store).await.unwrap();
        assert_eq!(report.repos_migrated, 0);
        assert_eq!(report.repos_already_migrated, 0);
        assert_eq!(report.repos_with_synth_errors, 0);
    }

    #[tokio::test]
    async fn run_closes_lingering_rr_issue_on_merged_patch() {
        let store = MemoryStore::new();
        let actor = ActorRef::test();

        let (patch_id, _) = store
            .add_patch(make_patch(DomainPatchStatus::Merged), &actor)
            .await
            .unwrap();
        let (parent_id, _) = store
            .add_issue(make_parent_task(&patch_id), &actor)
            .await
            .unwrap();
        let (rr_id, _) = store
            .add_issue(
                make_review_request_issue(&patch_id, &parent_id, IssueStatus::Open),
                &actor,
            )
            .await
            .unwrap();
        let (mr_id, _) = store
            .add_issue(
                make_merge_request_issue(&patch_id, &parent_id, IssueStatus::Open),
                &actor,
            )
            .await
            .unwrap();

        let report = run(&store).await.unwrap();
        assert_eq!(report.rr_issues_dropped, 1);
        assert_eq!(report.mr_issues_closed_or_failed, 1);

        assert_eq!(
            store.get_issue(&rr_id, false).await.unwrap().item.status,
            IssueStatus::Dropped
        );
        assert_eq!(
            store.get_issue(&mr_id, false).await.unwrap().item.status,
            IssueStatus::Closed
        );
    }

    #[tokio::test]
    async fn run_fails_mr_issue_on_closed_patch() {
        let store = MemoryStore::new();
        let actor = ActorRef::test();

        let (patch_id, _) = store
            .add_patch(make_patch(DomainPatchStatus::Closed), &actor)
            .await
            .unwrap();
        let (parent_id, _) = store
            .add_issue(make_parent_task(&patch_id), &actor)
            .await
            .unwrap();
        let (mr_id, _) = store
            .add_issue(
                make_merge_request_issue(&patch_id, &parent_id, IssueStatus::Open),
                &actor,
            )
            .await
            .unwrap();

        let report = run(&store).await.unwrap();
        assert_eq!(report.mr_issues_closed_or_failed, 1);

        assert_eq!(
            store.get_issue(&mr_id, false).await.unwrap().item.status,
            IssueStatus::Failed
        );
    }

    #[tokio::test]
    async fn run_leaves_issues_for_open_patches_alone() {
        let store = MemoryStore::new();
        let actor = ActorRef::test();

        let (patch_id, _) = store
            .add_patch(make_patch(DomainPatchStatus::Open), &actor)
            .await
            .unwrap();
        let (parent_id, _) = store
            .add_issue(make_parent_task(&patch_id), &actor)
            .await
            .unwrap();
        let (rr_id, _) = store
            .add_issue(
                make_review_request_issue(&patch_id, &parent_id, IssueStatus::Open),
                &actor,
            )
            .await
            .unwrap();

        let report = run(&store).await.unwrap();
        assert_eq!(report.rr_issues_dropped, 0);
        assert_eq!(report.mr_issues_closed_or_failed, 0);

        assert_eq!(
            store.get_issue(&rr_id, false).await.unwrap().item.status,
            IssueStatus::Open
        );
    }

    #[tokio::test]
    async fn run_is_idempotent_for_close_up_pass() {
        let store = MemoryStore::new();
        let actor = ActorRef::test();
        let (patch_id, _) = store
            .add_patch(make_patch(DomainPatchStatus::Merged), &actor)
            .await
            .unwrap();
        let (parent_id, _) = store
            .add_issue(make_parent_task(&patch_id), &actor)
            .await
            .unwrap();
        store
            .add_issue(
                make_review_request_issue(&patch_id, &parent_id, IssueStatus::Open),
                &actor,
            )
            .await
            .unwrap();

        let report1 = run(&store).await.unwrap();
        assert_eq!(report1.rr_issues_dropped, 1);
        assert_eq!(report1.issues_already_terminal, 0);

        let report2 = run(&store).await.unwrap();
        assert_eq!(report2.rr_issues_dropped, 0);
        assert_eq!(report2.issues_already_terminal, 1);
    }
}
