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
2. Verify the main landing page loads with the sidebar visible
   - Sidebar should show the "All chats" entry and a "Workspace" section containing Issues, Patches, Sessions, Documents, Agents, Repositories, and Secrets
3. Verify the landing page (root `/`) displays the issues list (table or list view)
   - Take an accessibility snapshot to confirm key UI elements
4. If any issues exist, click on one to open the **Issue Detail** page (`/issues/:issueId`)
   - Verify the issue detail page loads with title, description, status, and activity log
   - Navigate back to the issues list
5. Navigate to the **All chats** entry (`/chat`) via the sidebar
   - Verify the page loads and displays the conversations list (or empty state)
   - Take an accessibility snapshot to confirm key UI elements (New Chat button, etc.)
6. Navigate to the **Documents** page (`/documents`) via the sidebar
   - Verify the page loads and displays the documents table or list
7. Navigate to the **Repositories** page (`/repositories`) via the sidebar
   - Verify the page loads and displays the repositories list (or empty state)
8. Navigate to the **Agents** page (`/agents`) via the sidebar
   - Verify the page loads and displays the agents list (or empty state)
9. Navigate to the **Secrets** page (`/secrets`) via the sidebar
   - Verify the page loads and displays the secrets list (or empty state)
10. Take a screenshot of each page for visual verification

## Expected Results

- All key pages load without errors: issues list (root `/`), issue detail, chat (conversations list), documents, repositories, agents, and secrets
- The repositories, agents, and secrets pages each load independently with their own content
- Each page displays its primary content area (tables, lists, or empty-state messages)
- Navigation between pages works correctly via the sidebar
- No JavaScript errors, broken layouts, or missing UI components on any page
- Page titles and headings are correct for each section
