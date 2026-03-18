# Scenario: Dashboard Navigation

**ID:** dashboard-navigation
**Category:** core
**Priority:** P0
**Prerequisites:** Server running (server-init scenario passed)
**Estimated duration:** 3 minutes

## Description

Verify that all key dashboard pages load correctly without errors. This scenario navigates through the main sections of the Hydra dashboard and checks that each page renders its expected content.

## Steps (via dashboard)

1. Navigate to `http://localhost:8080`
2. Verify the main landing page loads with navigation elements visible
3. Navigate to the **Issues** list page
   - Verify the page loads and displays the issues table or list
   - Take an accessibility snapshot to confirm key UI elements
4. If any issues exist, click on one to open the **Issue Detail** page
   - Verify the issue detail page loads with title, description, status, and activity log
   - Navigate back to the issues list
5. Navigate to the **Patches** list page
   - Verify the page loads and displays the patches table or list
6. Navigate to the **Repositories** list page
   - Verify the page loads and displays the repositories table or list
7. Navigate to the **Documents** list page
   - Verify the page loads and displays the documents table or list
8. Navigate to the **Agents** list page
   - Verify the page loads and displays the agents table or list
9. Take a screenshot of each page for visual verification

## Expected Results

- All seven key pages load without errors: landing, issues list, issue detail, patches list, repositories list, documents list, agents list
- Each page displays its primary content area (tables, lists, or empty-state messages)
- Navigation between pages works correctly (links and routing function properly)
- No JavaScript errors, broken layouts, or missing UI components on any page
- Page titles and headings are correct for each section
