# Scenario: Reviewer Agent Reviews a Patch

**ID:** reviewer-agent
**Category:** agent-coordination
**Priority:** P1
**Prerequisites:** Server running (server-init scenario passed), at least one repository added (add-github-repo scenario passed), P0 scenarios and pm-agent-breakdown completed, reviewer agent configured as active on the repository
**Estimated duration:** 15 minutes

## Description

Verify that after an SWE agent creates a patch for an issue, the reviewer agent picks it up and reviews it. This tests the automated code review workflow through the dashboard. The reviewer agent must be configured to be active on the repository before running this scenario.

## Steps (via dashboard)

1. Navigate to the issues page at `http://localhost:8080`
2. Click the "Create issue" button
3. Fill in the issue creation form:
   - Title: "Add a .gitignore file to hydra-test-fixture"
   - Description: "Create a .gitignore file in the dourolabs/hydra-test-fixture repo with common entries for Node.js and Python projects."
   - Repository: dourolabs/hydra-test-fixture
   - Assignee: swe
4. Submit the issue form
5. Click on the newly created issue to open the issue detail page
6. Wait for the SWE agent to start processing the issue (poll for session activity)
7. Monitor until the SWE agent creates a patch (check the issue detail page and patches page)
8. Once a patch is created, navigate to the patches page
9. Find and click on the patch created by the SWE agent
10. Wait for the reviewer agent to pick up the patch and start a review session
11. Monitor the patch detail page for review activity:
    - Review comments appearing on the patch
    - Review status changing (e.g., approved or changes requested)
12. Verify the review is visible on the patch detail page

## Expected Results

- The SWE agent processes the issue and creates a patch
- The patch is visible in the patches list in the dashboard
- The reviewer agent automatically picks up the patch for review
- Review comments or approval appear on the patch detail page
- The review status is clearly indicated in the dashboard
- The full flow (issue -> SWE patch -> reviewer review) completes without manual intervention
