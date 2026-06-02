# User Scenarios

Central catalog of user scenarios for E2E validation. Each scenario has a unique tag
that maps to one or more Playwright tests via `@tag` annotations. Run a subset with
`pnpm e2e -- --grep @auth` (or any tag).

## Authentication

- `@auth:login` — User can log in with a valid API token
- `@auth:redirect` — Unauthenticated user is redirected to login
- `@auth:logout` — User can log out and is redirected to login

## Dashboard

- `@dashboard:view` — User can see the issue list on the dashboard (planned)
- `@dashboard:inbox` — Default `your-issues` view shows the logged-in user's own issues (creator = user), including dropped/closed states
- `@dashboard:search` — Issues list exposes both a free-text search box (server-side `?q=`) and a FilterBar; users can search by text or open the add-filter menu and pick a property filter (Status). Both surfaces persist their state to the URL.

## Navigation

- `@nav:sidebar` — User can navigate between pages using the sidebar
- `@nav:deep-link` — User can navigate directly to a page via URL
- `@nav:back-button` — Browser back button works correctly
- `@nav:sidebar-toggle` — User can hide and restore the sidebar; state persists across reloads
- `@nav:header-create-menu` — Header `+` button opens a menu with New issue / New conversation; selection invokes the matching action and closes the menu
- `@nav:tooltip-viewport` — Header tooltips stay within viewport at desktop and mobile sizes

## Issues

- `@issues:view-detail` — User can view an issue's description, metadata, and progress
- `@issues:update-status` — User can change an issue's status
- `@issues:create` — User can create a new issue
- `@issues:navigate-tabs` — User can navigate between Related, Activity, and Details tabs in the issue right panel

## Labels

- `@labels:display` — Labels are displayed on issue detail
- `@labels:create-with` — User can create an issue with existing and new labels
- `@labels:edit` — User can add and remove labels on an existing issue

## Patches

- `@patches:view-detail` — User can view a patch's details and metadata
- `@patches:navigate` — User can navigate to a patch from an issue

## Documents

- `@documents:list` — User can view the documents list
- `@documents:view-detail` — User can view a document's content
- `@documents:up-one-level` — Reader pane shows an "Up to <parent>" entry at non-root paths that navigates to the parent folder; absent at the root

## Chat

- `@chat:sidebar` — Clicking a chat row in the sidebar navigates to /chat/&lt;id&gt;
- `@chat:default-mine` — `/chat` defaults to the logged-in user's chats (Mine); toggle flips to All (`?scope=all`) and back, the Chats query carries `creator=<me>` by default
- `@chat:transcript-source` — Chat detail page renders from `SessionEvent` (`data-transcript-source="session_events"` on the message list). Across a 2-session resumption chain the merged transcript renders in chronological order and the per-session fan-out hits each session's `/v1/sessions/:id/events`.
- `@chat:conversation-switch` — Soft-navigating directly between two conversations (sidebar click on `/chat/A` → `/chat/B`) shows only the new conversation's messages: the previous transcript does not leak, and a not-yet-reconciled optimistic message sent in the previous conversation does not appear in the new one.
- `@chat:activity-status` — After the user sends a message, a transient activity indicator appears below the message thread (`Thinking…`), transitions through at least one `ToolUse` label as the worker emits `tool_use` events, and disappears once an `AssistantMessage` lands. The indicator is not part of the transcript history.
- `@chat:reference-preview-cards` — A chat message containing `[[id]]` references for issues / patches / documents / sessions / conversations renders a preview card per unique referenced object at the end of the message, in source order, deduplicated. Inline `[[id]]` rendering in the message body is unchanged.

## Repositories

- `@repos:edit-merge-policy` — User can view, edit, clear, and round-trip a repository's `merge_policy` via the Repository edit modal's JSON textarea, with inline error on invalid JSON

## Error Handling

- `@errors:404` — User sees a not-found message for non-existent entities
- `@errors:server-error` — User sees an error message when the server returns 500

## Sessions

- `@sessions:kill` — User can kill a running session with confirmation
- `@sessions:filter-bar` — Sessions list toolbar uses the shared `<FilterBar>`. On first visit a creator chip is auto-added for the logged-in user (`?creator=users/<me>`) and `listSessions` narrows by creator; opening the + Filter menu, picking Status → running writes `?status=running` and refetches with the new server params; removing the auto-added creator chip strips `?creator=` from the URL and refetches without it; legacy `?scope=mine` redirects to the creator chip on first paint and the legacy param is stripped.

## Mobile Viewport

- `@mobile:nav` — Navigation works correctly on mobile viewport
- `@mobile:dashboard` — Dashboard is usable on mobile viewport
- `@mobile:issue-detail` — Issue detail page shows Overview / Related / Activity / Details top tabs on mobile; Overview is default; Related surfaces parents/children/patches/documents; Activity surfaces the timeline; Details surfaces rail content (Status, Created, Labels, etc.). Desktop hides the mobile bar and uses the right-rail sub-tabs.
- `@mobile:issue-detail-overflow` — Issue detail page fits the 375px viewport (no document-level horizontal overflow) and the SessionList at the bottom of Overview is reachable via vertical scroll.
- `@mobile:list-overflow` — List pages (sessions, patches, issues, chats, repositories, agents, secrets) have no document-level horizontal overflow at 375 px.
- `@mobile:login` — Login page is usable on mobile viewport
- `@mobile:chat-scroll` — Chat header stays visible on mobile and the message list owns scroll (no page-level snap-to-bottom)
- `@mobile:chat-tabs` — Right-panel content (Related, Details) is reachable via top tabs on the chat page mobile viewport; the Chat tab is default and the message-list scroll is not regressed
- `@mobile:breadcrumbs` — At ≤768px the breadcrumb trail collapses to only the trailing (current) crumb; at desktop widths the full trail (ancestors + current) remains visible
- `@mobile:chat-composer` — Chat composer textarea has ≥16px font-size at mobile widths (prevents iOS Safari focus-zoom) and a background distinct from the page background in both dark and light themes
- `@mobile:chat-bottom-safe-area` — At mobile widths the chat detail composer sits clear of the iOS Safari home-indicator: the AppLayout main scroll container's bottom padding scales with `env(safe-area-inset-bottom)`
- `@mobile:issue-detail-bottom-safe-area` — At mobile widths the issue detail SessionList sits clear of the iOS Safari home-indicator: the AppLayout main scroll container's bottom padding scales with `env(safe-area-inset-bottom)`, and the list remains reachable via vertical scroll
- `@mobile:documents-single-pane` — At ≤768px the documents page collapses to a single pane (the reader pane); the left document tree (`aside[aria-label="Document tree"]`) is hidden via `display: none`
- `@mobile:chat-header-meta` — At mobile widths the chat-details subheading renders "started Xm ago" with a visible space and the meta row wraps cleanly (no separator at line edges, no overlapping characters)

## Responsive Layout

- `@layout:responsive` — Main content renders with a non-zero box and is not occluded at every supported viewport width (1440 → 375 px)
- `@layout:responsive-drawer` — Mobile drawer opens via hamburger and dismisses via backdrop; desktop hamburger collapses sidebar without hiding main content
