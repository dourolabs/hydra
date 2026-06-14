mod harness;

use anyhow::{Context, Result};
use hydra_common::{
    api::v1::comments::ListCommentsQuery,
    issues::{
        IssueDependencyType, IssueInput, IssueType, SessionSettings, SubmitFormRequest,
        UpsertIssueRequest,
    },
    principal::Principal,
    projects::{
        ProjectKey, ProjectRef, StatusDefinition, StatusKey, StatusOnEnter, UpsertProjectRequest,
    },
    users::Username,
    IssueId, RepoName, SessionId,
};
use hydra_server::domain::{actors::ActorRef, documents::Document};
use std::{collections::HashMap, str::FromStr};

const REPO: &str = "test-org/status-flow";
const FORM_PATH: &str = "/forms/test_approve_reject.yaml";
const FORM_PROMPT: &str = "Approve or reject this issue";

const FORM_BODY: &str = r#"prompt: "Approve or reject this issue"
fields:
  - key: review_comment
    label: Review Comment
    input:
      type: textarea
      placeholder: ""
      rows: 4
actions:
  - id: approve
    label: Approve
    style: primary
    requires: []
    effect:
      type: update_issue
      status: done
  - id: reject
    label: Reject
    style: danger
    requires: [review_comment]
    effect:
      type: update_issue
      status: in-development
      add_comment_from: review_comment
"#;

fn agent_principal(name: &str) -> Principal {
    Principal::Agent {
        name: hydra_common::api::v1::agents::AgentName::try_new(name)
            .expect("agent name should validate"),
    }
}

fn user_principal(name: &str) -> Principal {
    Principal::User {
        name: hydra_common::api::v1::users::Username::try_new(name)
            .expect("username should validate"),
    }
}

fn status_def(
    key: &str,
    label: &str,
    unblocks_parents: bool,
    unblocks_dependents: bool,
    on_enter: Option<StatusOnEnter>,
) -> StatusDefinition {
    StatusDefinition::new(
        StatusKey::try_new(key).unwrap(),
        label.to_string(),
        "#abcdef".parse().unwrap(),
        unblocks_parents,
        unblocks_dependents,
        false,
        on_enter,
    )
}

/// Status list for the test project. `escalation` is declared to match the
/// spec but neither flow transitions into it.
fn engineering_statuses() -> Vec<StatusDefinition> {
    vec![
        status_def(
            "backlog",
            "Backlog",
            false,
            false,
            Some(StatusOnEnter::new(Some(agent_principal("pm")), None)),
        ),
        status_def(
            "in-development",
            "In Development",
            false,
            false,
            Some(StatusOnEnter::new(Some(agent_principal("swe")), None)),
        ),
        status_def(
            "in-review",
            "In Review",
            false,
            false,
            Some(StatusOnEnter::new(Some(agent_principal("reviewer")), None)),
        ),
        status_def("escalation", "Escalation", false, false, None),
        status_def(
            "merging",
            "Merging",
            false,
            false,
            Some(StatusOnEnter::new(
                // `StatusOnEnter.assign_to` is a static `Principal`; in this
                // single-user test the `default` user stands in for "issue
                // creator" since there's only one human in play.
                Some(user_principal("default")),
                Some(FORM_PATH.parse().unwrap()),
            )),
        ),
        status_def("done", "Done", true, true, None),
    ]
}

async fn setup(harness: &harness::TestHarness) -> Result<IssueId> {
    let user = harness.default_user();

    let form_doc = Document {
        title: "Approve/Reject form".to_string(),
        body_markdown: FORM_BODY.to_string(),
        path: Some(FORM_PATH.parse().unwrap()),
        deleted: false,
    };
    harness
        .store()
        .add_document(form_doc, &ActorRef::test())
        .await
        .context("seed approve/reject form")?;

    let upsert = UpsertProjectRequest::new(
        ProjectKey::try_new("engineering").unwrap(),
        "Engineering".to_string(),
    );
    let project_resp = user
        .client()
        .create_project(&upsert)
        .await
        .context("create engineering project")?;
    let project_ref = ProjectRef::Id(project_resp.project_id.clone());
    for status in engineering_statuses() {
        user.client()
            .create_project_status(&project_ref, &status)
            .await
            .context("seed engineering status")?;
    }

    let repo = RepoName::from_str(REPO)?;
    let mut session_settings = SessionSettings::default();
    session_settings.repo_name = Some(repo);
    let parent = IssueInput::new(
        IssueType::Task,
        "Status-flow parent".to_string(),
        "drive a project-status flow end-to-end".to_string(),
        Username::from("default"),
        StatusKey::try_new("backlog").unwrap(),
        project_resp.project_id,
        None,
        Some(session_settings),
        Vec::new(),
        Vec::new(),
        false,
        None,
        None,
    );
    let resp = user
        .client()
        .create_issue(&UpsertIssueRequest::new(parent, None))
        .await
        .context("create parent issue")?;
    Ok(resp.issue_id)
}

/// Drive PM → SWE → reviewer → `merging`, where the form is attached and the
/// issue waits on the user. Returns the child id plus every session id spawned
/// for that child so far (so the reject path can detect a brand-new SWE spawn
/// by set-difference rather than guessing the original ids).
async fn drive_to_merging(
    harness: &harness::TestHarness,
    parent_id: &IssueId,
) -> Result<(IssueId, Vec<SessionId>)> {
    let user = harness.default_user();

    let pm_sessions = harness.await_sessions(parent_id, 1).await?;
    assert_eq!(pm_sessions.len(), 1, "PM session should spawn for parent");
    let parent_after_pm = user.get_issue(parent_id).await?;
    assert_eq!(
        parent_after_pm.issue.assignee.as_ref(),
        Some(&agent_principal("pm")),
        "parent assignee should be pm after apply_status_on_enter"
    );

    harness
        .run_worker(
            &pm_sessions[0],
            vec![&format!(
                "hydra issues create 'status-flow child task' \
                 --type task --project engineering --status in-development \
                 --deps child-of:{parent_id} --repo-name {REPO}"
            )],
        )
        .await?;

    let child_id = find_child_of(&user.list_issues().await?.issues, parent_id)
        .context("PM should have created exactly one child")?;

    let swe_sessions = harness.await_sessions(&child_id, 1).await?;
    assert_eq!(swe_sessions.len(), 1, "SWE session should spawn for child");
    let child_after_create = user.get_issue(&child_id).await?;
    assert_eq!(
        child_after_create.issue.assignee.as_ref(),
        Some(&agent_principal("swe")),
        "child assignee should be swe after apply_status_on_enter"
    );

    harness
        .run_worker(
            &swe_sessions[0],
            vec![&format!(
                "hydra issues update {child_id} --status in-review"
            )],
        )
        .await?;
    let child_in_review = user.get_issue(&child_id).await?;
    assert_eq!(
        child_in_review.issue.assignee.as_ref(),
        Some(&agent_principal("reviewer")),
        "child assignee should be reviewer after in-review on_enter"
    );

    let reviewer_sessions = harness.await_sessions(&child_id, 2).await?;
    let reviewer_session = reviewer_sessions
        .iter()
        .find(|id| !swe_sessions.contains(id))
        .cloned()
        .context("reviewer session should be a new id distinct from SWE")?;

    harness
        .run_worker(
            &reviewer_session,
            vec![&format!("hydra issues update {child_id} --status merging")],
        )
        .await?;
    let child_merging = user.get_issue(&child_id).await?;
    assert_eq!(
        child_merging.issue.assignee.as_ref(),
        Some(&user_principal("default")),
        "child assignee should be the `default` user after merging on_enter"
    );
    let form = child_merging
        .issue
        .form
        .as_ref()
        .context("merging.on_enter should attach the form")?;
    assert_eq!(
        form.prompt, FORM_PROMPT,
        "attached form prompt should match the seeded YAML"
    );
    assert!(
        form.actions.iter().any(|a| a.id == "approve")
            && form.actions.iter().any(|a| a.id == "reject"),
        "form should declare both approve and reject actions"
    );

    Ok((child_id, reviewer_sessions))
}

fn find_child_of(
    summaries: &[hydra_common::issues::IssueSummaryRecord],
    parent: &IssueId,
) -> Option<IssueId> {
    summaries
        .iter()
        .find(|s| {
            s.issue
                .dependencies
                .iter()
                .any(|d| d.dependency_type == IssueDependencyType::ChildOf && &d.issue_id == parent)
        })
        .map(|s| s.issue_id.clone())
}

#[tokio::test]
async fn status_based_flow_approve_path() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo(REPO)
        .with_agent("pm", "Plan and coordinate tasks")
        .with_agent("swe", "Implement code changes")
        .with_agent("reviewer", "Review code")
        .build()
        .await?;

    let parent_id = setup(&harness).await?;
    let (child_id, _sessions_so_far) = drive_to_merging(&harness, &parent_id).await?;

    let user = harness.default_user();
    user.client()
        .submit_form(
            &child_id,
            &SubmitFormRequest::new("approve".to_string(), HashMap::new()),
        )
        .await
        .context("submit approve form")?;

    let child_after = user.get_issue(&child_id).await?;
    assert_eq!(
        child_after.issue.status.key.as_str(),
        "done",
        "approve action should transition child to `done`"
    );
    assert!(
        child_after.issue.status.unblocks_parents,
        "`done` must mark unblocks_parents so the parent can close"
    );
    let approve_comments = user
        .client()
        .list_issue_comments(&child_id, &ListCommentsQuery::default())
        .await
        .context("list comments after approve")?;
    assert!(
        approve_comments.comments.is_empty(),
        "approve action should not post a comment (no add_comment_from declared)"
    );

    Ok(())
}

#[tokio::test]
async fn status_based_flow_reject_path() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo(REPO)
        .with_agent("pm", "Plan and coordinate tasks")
        .with_agent("swe", "Implement code changes")
        .with_agent("reviewer", "Review code")
        .build()
        .await?;

    let parent_id = setup(&harness).await?;
    let (child_id, sessions_before_reject) = drive_to_merging(&harness, &parent_id).await?;

    let user = harness.default_user();
    let mut values = HashMap::new();
    values.insert(
        "review_comment".to_string(),
        serde_json::Value::String("needs work".to_string()),
    );
    user.client()
        .submit_form(
            &child_id,
            &SubmitFormRequest::new("reject".to_string(), values),
        )
        .await
        .context("submit reject form")?;

    let child_after = user.get_issue(&child_id).await?;
    assert_eq!(
        child_after.issue.status.key.as_str(),
        "in-development",
        "reject action should transition child back to `in-development`"
    );
    let reject_comments = user
        .client()
        .list_issue_comments(&child_id, &ListCommentsQuery::default())
        .await
        .context("list comments after reject")?;
    assert_eq!(
        reject_comments.comments.len(),
        1,
        "reject should post exactly one comment via add_comment_from"
    );
    assert_eq!(
        reject_comments.comments[0].body, "needs work",
        "reject should post the review_comment field as the comment body"
    );
    assert_eq!(
        child_after.issue.assignee.as_ref(),
        Some(&agent_principal("swe")),
        "in-development.on_enter should reassign to SWE on re-entry"
    );

    // A re-entry to `in-development` must produce a brand-new SWE session via
    // spawn_sessions. Same shape as scenario_planning_agent_flow.rs lines 138–147.
    let sessions_after = harness
        .await_sessions(&child_id, sessions_before_reject.len() + 1)
        .await?;
    let new_session = sessions_after
        .iter()
        .find(|id| !sessions_before_reject.contains(id))
        .cloned()
        .context("expected a brand-new SWE session after reject re-spawn")?;
    assert!(
        !sessions_before_reject.contains(&new_session),
        "re-spawned SWE session must not equal any session id from before the reject"
    );

    Ok(())
}
