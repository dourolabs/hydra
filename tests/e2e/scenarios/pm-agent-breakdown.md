# Scenario: PM Agent Task Breakdown

**ID:** pm-agent-breakdown
**Category:** agent-coordination
**Priority:** P1
**Prerequisites:** Server running (server-init scenario passed), at least one repository added (add-github-repo scenario passed)
**Estimated duration:** 10 minutes

## Description

Create a high-level issue in the dashboard and verify that the PM agent picks it up and breaks it into child tasks. This tests the PM agent's ability to decompose work items into actionable sub-tasks visible in the dashboard.

## Steps (via dashboard)

1. Navigate to the issues page at `http://localhost:8080`
2. Click the "Create issue" button
3. Fill in the issue creation form:
   - Title: "Improve documentation for hydra-test-fixture"
   - Description: "The hydra-test-fixture repo needs documentation improvements. Please update the README to add a Troubleshooting section, add a Code of Conduct section to CONTRIBUTING.md, and create a CHANGELOG.md file."
4. Submit the issue form (do not assign to a specific agent -- let PM handle it)
5. Click on the newly created issue to open the issue detail page
6. Wait for the PM agent to pick up the issue (poll the issue detail page for session activity)
7. Monitor the issue detail page for child issues to appear
8. Once child issues are visible, verify that:
   - Multiple child tasks have been created
   - Each child task has a clear, actionable title
   - Child tasks are assigned to appropriate agents (e.g., swe)
9. Click on each child issue to verify it has a description and is properly linked to the parent

## Expected Results

- The high-level issue is created and visible in the dashboard
- The PM agent starts a session to process the issue
- The PM agent creates multiple child tasks that break down the high-level issue
- Each child task has a descriptive title and is assigned to an agent
- Child tasks are visible on the parent issue's detail page
- Navigating to each child issue shows the parent relationship
