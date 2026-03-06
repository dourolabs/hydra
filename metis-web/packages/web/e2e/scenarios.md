# User Scenarios

Central catalog of user scenarios for E2E validation. Each scenario has a unique tag
that maps to one or more Playwright tests via `@tag` annotations. Run a subset with
`pnpm e2e -- --grep @auth` (or any tag).

## Authentication

- `@auth:login` — User can log in with a valid API token
- `@auth:redirect` — Unauthenticated user is redirected to login
- `@auth:logout` — User can log out and is redirected to login
- `@auth:invalid-token` — User sees error when logging in with invalid token

## Dashboard

- `@dashboard:view` — User can see the issue list on the dashboard (planned)
- `@dashboard:inbox` — User can filter dashboard to inbox items
- `@dashboard:search` — User can search for issues by title

## Navigation

- `@nav:sidebar` — User can navigate between pages using the sidebar
- `@nav:deep-link` — User can navigate directly to a page via URL
- `@nav:back-button` — Browser back button works correctly

## Issues

- `@issues:view-detail` — User can view an issue's description, metadata, and progress
- `@issues:update-status` — User can change an issue's status
- `@issues:create` — User can create a new issue
- `@issues:navigate-tabs` — User can navigate between Related Issues, Jobs, Patches, Activity, and Metadata tabs

## Labels

- `@labels:display` — Labels are displayed on dashboard item rows and issue detail
- `@labels:create-with` — User can create an issue with existing and new labels
- `@labels:edit` — User can add and remove labels on an existing issue

## Patches

- `@patches:view-detail` — User can view a patch's details and metadata
- `@patches:navigate` — User can navigate to a patch from an issue

## Documents

- `@documents:list` — User can view the documents list
- `@documents:view-detail` — User can view a document's content

## Error Handling

- `@errors:404` — User sees a not-found message for non-existent entities
- `@errors:server-error` — User sees an error message when the server returns 500

## Jobs

- `@jobs:kill` — User can kill a running job with confirmation

## Mobile Viewport

- `@mobile:nav` — Navigation works correctly on mobile viewport
- `@mobile:dashboard` — Dashboard is usable on mobile viewport
- `@mobile:issue-detail` — Issue detail page is usable on mobile viewport
- `@mobile:login` — Login page is usable on mobile viewport (planned)
