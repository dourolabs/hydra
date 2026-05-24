# Scenario: Basic Issue Lifecycle

**ID:** basic-issue-lifecycle
**Category:** core
**Priority:** P0
**Prerequisites:** Server running (server-init scenario passed), at least one repository added (add-github-repo scenario passed)
**Estimated duration:** 10 minutes

## Description

Create an issue via the dashboard assigned to an SWE agent, wait for the agent to run and process the issue, and verify the issue reaches a closed state. All interactions are performed through the dashboard UI.

## Steps (via dashboard)

1. Navigate to the issues page at `http://localhost:8080`
2. Click the "Create issue" button
3. Fill in the issue creation form:
   - Title: "Make a small improvement to the hydra-test-fixture repo"
   - Description: "Make any small, low-risk improvement to the dourolabs/hydra-test-fixture repo at your discretion — for example, a typo fix, a minor wording polish, a tiny docs improvement, or an obviously-harmless cleanup. Use your judgment to pick the change. The goal is to submit a PR with a trivial change, not to make any specific edit."
   - Repository: dourolabs/hydra-test-fixture
   - Assignee: swe
4. Submit the issue form
5. Verify the issue appears in the issues list
6. Click on the newly created issue to open the issue detail page
7. Verify the issue status is "open"
8. Wait for an agent session to be created for this issue (poll the issue detail page)
9. Monitor the issue detail page for status transitions (open -> in-progress -> closed)
10. Wait until the issue status reaches "closed" (allow up to 10 minutes for agent execution)
11. Verify that the issue detail page shows the final "closed" status
12. Verify a patch was produced by navigating to the patches page and filtering for this issue.
    Assert that at least one patch is listed for this issue and that the patch has a non-empty
    diff (any non-zero number of changed lines counts as a pass). The scenario is intentionally
    open-ended about *what* the SWE changes — we only care that the patch-submission path was
    exercised with real content.
13. Verify the agent reported session-level token-usage statistics on completion:
    - Find the session that ran for this issue. From the dashboard, navigate to the
      sessions page and locate the session whose `spawned_from` matches this issue id.
      Alternatively, query the API directly:
      `curl -s http://localhost:8080/v1/sessions?spawned_from=<ISSUE_ID> | jq '.sessions[0].session_id'`
    - Fetch the full session record:
      `curl -s http://localhost:8080/v1/sessions/<SESSION_ID> | jq '.session.usage'`
    - Assert the `usage` field is present (non-null) and that both `input_tokens > 0`
      and `output_tokens > 0`. A successful Claude run consumes at least a few hundred
      input tokens and emits at least a handful of output tokens, so any positive
      integer counts as a pass; zero or null on either field is a failure.

## Expected Results

- The issue is created successfully with the correct title, description, and assignee
- An agent session starts within a reasonable time
- The issue transitions through expected states: open -> in-progress -> closed
- The issue reaches "closed" status, indicating the agent completed the task
- The agent submits at least one patch with a non-empty diff, visible in the patches list
- The issue detail page shows a complete activity log of the lifecycle
- The session record exposes a `usage` object whose `input_tokens` and `output_tokens`
  are both strictly positive (this is the assertion added in step 13 — see
  `hydra/src/worker/report.rs` `RunReport.usage` and `hydra-server` `transition_task_to_completion`
  for where this data is captured and persisted).
