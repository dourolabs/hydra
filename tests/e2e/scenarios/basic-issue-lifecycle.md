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
   - Title: "Add a Code of Conduct section to the CONTRIBUTING.md in hydra-test-fixture"
   - Description: "Update the CONTRIBUTING.md file in the dourolabs/hydra-test-fixture repo to add a Code of Conduct section with guidelines for respectful collaboration."
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
12. Check if any patches were created by navigating to the patches page and filtering for this issue
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
- If the agent created a patch, it is visible in the patches list
- The issue detail page shows a complete activity log of the lifecycle
- The session record exposes a `usage` object whose `input_tokens` and `output_tokens`
  are both strictly positive (this is the assertion added in step 13 — see
  `hydra/src/worker/report.rs` `RunReport.usage` and `hydra-server` `transition_task_to_completion_with_usage`
  for where this data is captured and persisted).
