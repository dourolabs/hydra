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
2. Verify the main landing page (Dashboard) loads with the icon sidebar visible
   - Sidebar should show Dashboard, Documents, and Settings navigation icons
3. Verify the Dashboard page displays the issues list (table or list view)
   - Take an accessibility snapshot to confirm key UI elements
4. If any issues exist, click on one to open the **Issue Detail** page (`/issues/:issueId`)
   - Verify the issue detail page loads with title, description, status, and activity log
   - Navigate back to the Dashboard
5. Navigate to the **Documents** page (`/documents`) via the sidebar
   - Verify the page loads and displays the documents table or list
6. Navigate to the **Settings** page (`/settings`) via the sidebar
   - Verify the page loads and displays the **Repositories** section
   - Verify the page displays the **Agents** section
   - Verify the page displays the **Secrets** section
7. Take a screenshot of each page for visual verification

## Expected Results

- All key pages load without errors: Dashboard (issues list), issue detail, documents, and settings
- The Settings page shows Repositories, Agents, and Secrets sections
- Each page displays its primary content area (tables, lists, or empty-state messages)
- Navigation between pages works correctly via the icon sidebar
- No JavaScript errors, broken layouts, or missing UI components on any page
- Page titles and headings are correct for each section
