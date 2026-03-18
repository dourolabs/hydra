# Scenario: Add GitHub Repository via Dashboard

**ID:** add-github-repo
**Category:** core
**Priority:** P0
**Prerequisites:** Server running (server-init scenario passed)
**Estimated duration:** 5 minutes

## Description

Add a GitHub repository (dourolabs/hydra-test-fixture) to Hydra by creating an issue in the dashboard. This exercises the add-new-repo playbook end-to-end through the frontend UI. The tester creates the issue through the dashboard, then monitors until the repo appears in the repositories page.

## Steps (via dashboard)

1. Navigate to the issues page at `http://localhost:8080`
2. Click the "Create issue" button in the dashboard
3. Fill in the issue creation form:
   - Title: "Add repo dourolabs/hydra-test-fixture"
   - Description: "Add repo https://github.com/dourolabs/hydra-test-fixture.git"
4. Submit the issue form
5. Verify the issue appears in the issues list with the correct title
6. Click on the newly created issue to open the issue detail page
7. Wait for a PM agent session to start processing the issue (poll the issue detail page)
8. Monitor the issue detail page until child issues are created by the PM agent
9. Wait for child issues to reach terminal states (closed or completed)
10. Navigate to the repositories page in the dashboard
11. Verify that `dourolabs/hydra-test-fixture` appears in the repository list

## Expected Results

- The issue is created and visible in the issues list in the dashboard
- The issue detail page shows the correct title and description
- A PM agent session starts and processes the issue
- Child issues are created as part of the add-new-repo playbook execution
- All child issues eventually reach a terminal state
- The repositories page shows `dourolabs/hydra-test-fixture` as an added repository
- No errors or broken UI elements are visible throughout the flow
