# Scenario: Full Agent Handoff Flow

**ID:** agent-handoff
**Category:** agent-coordination
**Priority:** P1
**Prerequisites:** Server running (server-init scenario passed), at least one repository added (add-github-repo scenario passed)
**Estimated duration:** 20 minutes

## Description

Test the full agent coordination flow through the dashboard: create a high-level issue, verify the PM agent breaks it down into child tasks, the SWE agent implements changes and creates patches, and the reviewer agent reviews the patches. This validates the complete multi-agent handoff workflow.

## Steps (via dashboard)

1. Navigate to the issues page at `http://localhost:8080`
2. Click the "Create issue" button
3. Fill in the issue creation form:
   - Title: "Add CI configuration to hydra-test-fixture"
   - Description: "Set up a basic GitHub Actions CI workflow for the dourolabs/hydra-test-fixture repo. The workflow should run linting and tests on pull requests."
4. Submit the issue form (do not assign to a specific agent -- let PM handle it)
5. Click on the newly created issue to open the issue detail page

**Phase 1: PM Breakdown**
6. Wait for the PM agent to pick up the issue
7. Monitor the issue detail page until child tasks are created
8. Verify child tasks are visible and assigned to SWE agents

**Phase 2: SWE Implementation**
9. Click on a child task assigned to SWE
10. Monitor the child issue detail page for agent session activity
11. Wait for the SWE agent to complete work and create a patch
12. Navigate to the patches page and verify the patch is listed

**Phase 3: Reviewer Review**
13. Click on the patch to open the patch detail page
14. Wait for the reviewer agent to start reviewing the patch
15. Monitor for review comments or approval on the patch detail page
16. Verify the review completes with a clear status (approved or changes requested)

**Phase 4: Completion**
17. Navigate back to the parent issue detail page
18. Monitor until all child issues reach terminal states
19. Verify the parent issue eventually reaches a closed state

## Expected Results

- PM agent creates actionable child tasks from the high-level issue
- SWE agent implements changes and creates patches for child tasks
- Reviewer agent reviews the patches automatically
- The full workflow completes: PM breakdown -> SWE implementation -> Reviewer review
- All status transitions are visible in the dashboard
- The parent issue and child issues reach terminal states
- The dashboard shows a coherent activity log across all stages of the workflow
