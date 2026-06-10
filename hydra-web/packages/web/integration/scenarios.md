# User Scenarios

Central catalog of user scenarios for integration validation. Each scenario has a unique tag
that maps to one or more Playwright tests via `@tag` annotations. Run a subset with
`pnpm integration -- --grep @auth` (or any tag).

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
- `@issues:view-detail-archived` — Navigating to a soft-deleted (archived) issue id renders the detail page normally with all the usual content plus an "Archived" badge in the title row. The page's `getIssue` request carries `include_deleted=true` so the server returns the archived row. Non-archived issues do not render the Archived badge.
- `@issues:update-status` — User can change an issue's status
- `@issues:create` — User can create a new issue
- `@issues:navigate-tabs` — User can navigate between Related, Activity, and Details tabs in the issue right panel
- `@issues:filter-related-chat-narrows` — Issues list FilterBar can add a Related chat chip and pick a seeded conversation; the listIssues request goes out with `ids=` containing only `i-`-prefixed ids (no `d-`/`p-` leakage from chat→artifact `refers-to` edges) and the rendered rows are exactly the issues the seed says that conversation refers to. URL persists `?relatedChat=<id>` and reload rehydrates the chip + narrowed list.
- `@issues:filter-related-chat-no-flash` — Changing a rehydrated Related chat chip's selection (adding a second value) keeps the previous narrowed rows rendered until the new resolution lands: with the swap's `/v1/relations` call held by a test intercept, the rows container never empties to zero and neither the "Loading issues…" skeleton nor the empty state appears. Releasing the held call swaps in the new union of rows.
- `@issues:interactive-conversation` — When an issue has a spawned conversation (`Conversation.spawned_from == issueId`), the issue header surfaces a deep-link to `/chat/<conversation_id>`: labeled "Open Conversation" for `active`, "Resume Conversation" for `idle`, and absent for `closed`. The Related tab's Conversations subsection lists every linked conversation (live + historical) via `listConversations({ spawned_from })`. The target conversation's header in turn renders an "originated from [[issue_id]]" link back to the issue.
- `@issues:board-drag-reorder` — On the `/issues` Board layout, dragging a project bar with real-DOM mouse events fires exactly one `PUT /v1/projects/<id>` with the new numeric `priority` and the new order survives reload (the mock server returns projects sorted `priority ASC`, matching the real backend). Dragging a status column head fires sequential `PUT /v1/projects/<ref>/statuses/<key>` calls — one per status — each carrying the recomputed `position` (multiples of 100), and the new column order survives reload (statuses are sorted `position ASC` server-side).

## Labels

- `@labels:display` — Labels are displayed on issue detail
- `@labels:create-with` — User can create an issue with existing and new labels
- `@labels:edit` — User can add and remove labels on an existing issue

## Projects

- `@projects:create` — User can create a project with custom statuses from `/projects`; the new project lands in the list and is reachable at `/projects/<key>`.
- `@projects:badge` — Status badge on the issue list reflects the project's `StatusDefinition` (label, color) by reading `issue.resolved_status` straight from the API; the frontend performs no per-status resolution.
- `@projects:status-modal-options` — Status-update modal shows project-defined options for a project-scoped issue (fetched from `/v1/projects/:id/statuses`); every issue carries a real `project_id`, so the seeded default project (`j-defaul`) is fetched through the same route as any other project.
- `@projects:interactive-status` — Project editor exposes an "Interactive" checkbox alongside the existing status flags (`unblocks_parents`, `unblocks_dependents`, `cascades_to_children`). Toggling it on round-trips through the upsert request, and statuses with `interactive: true` render a small "interactive" annotation chip next to the status label in any `<StatusChip>` view.
- `@projects:details-rail-project-block` — The issue detail right-rail Details tab includes a Project row between Status and Assignee. The row renders a `<ProjectChip>` with the issue's resolved project key + name. Issues created without an explicit `project_id` are persisted against the seeded default project (`j-defaul`), and the chip renders that project.
- `@issues:blocked-tag` — The Details rail's Status row shows a mono "BLOCKED" tag next to the StatusChip when the issue has at least one `blocked-on` dependency target whose status is not `closed`. The tag is absent for issues with no open blockers.

## Patches

- `@patches:view-detail` — User can view a patch's details and metadata
- `@patches:navigate` — User can navigate to a patch from an issue
- `@patches:filter-bar` — Patches list toolbar uses the generic `<FilterBar>`. User can open + Filter, pick Status → Merged, the URL persists `?status=Merged`, the table narrows server-side (`listPatches` is called with `status=Merged`), and a page reload re-hydrates the chip from the URL.
- `@patches:filter-related-issue-narrows` — Patches list FilterBar can add a Related issue chip and pick one or more seeded issues; the listPatches request goes out with `ids=` containing only `p-`-prefixed ids and the rendered rows are exactly the patches the seed says those issues `has-patch` reference. URL persists `?relatedIssue=<csv>` and reload rehydrates the chip + narrowed list.
- `@patches:filter-related-issue-no-flash` — Changing a rehydrated Related issue chip's selection (adding a second value) keeps the previous narrowed rows rendered until the new resolution lands: with the swap's `/v1/relations` call held by a test intercept, the rows container never empties to zero and neither the "Loading patches…" skeleton nor the empty state appears. Releasing the held call swaps in the new union of rows.

## Documents

- `@documents:list` — User can view the documents list
- `@documents:view-detail` — User can view a document's content
- `@documents:up-one-level` — Reader pane shows an "Up to <parent>" entry at non-root paths that navigates to the parent folder; absent at the root

## Chat

- `@chat:sidebar` — Clicking a chat row in the sidebar navigates to /chat/&lt;id&gt;
- `@chat:default-mine` — `/chat` defaults to the logged-in user's chats (Mine): an auto-seeded `creator` FilterBar chip carries `creator=<me>` to the server on first paint and persists to `?creator=users/<me>` in the URL. Removing the chip flips to All-equivalent behaviour (no creator filter). Legacy `?scope=mine` / `?scope=all` URLs redirect to the FilterBar equivalent on first paint.
- `@chat:filter-bar` — Chats page toolbar exposes the generic FilterBar (`+ Filter`, chips, `Clear all`, summary) alongside a debounced free-text search box (`?q=`). Opening the menu and picking Status → active writes `?status=active` to the URL and narrows the `listConversations` server query; removing the auto-creator chip strips `?creator=` from the URL.
- `@chat:transcript-source` — Chat detail page renders from `SessionEvent` (`data-transcript-source="session_events"` on the message list). Across a 2-session resumption chain the merged transcript renders in chronological order and the per-session fan-out hits each session's `/v1/sessions/:id/events`.
- `@chat:conversation-switch` — Soft-navigating directly between two conversations (sidebar click on `/chat/A` → `/chat/B`) shows only the new conversation's messages: the previous transcript does not leak, and a not-yet-reconciled optimistic message sent in the previous conversation does not appear in the new one.
- `@chat:activity-status` — After the user sends a message, an inline activity line appears as the trailing transcript item inside the message list (`Thinking…`), transitions through at least one `ToolUse` label as the worker emits `tool_use` events (with a tool's `description` surfaced in the detail span), and settles into a `done`-state summary (e.g. `2 steps`) once an `AssistantMessage` lands. The line lives inside `ChatMessageList` and is preserved alongside the assistant reply so the user can review what happened.
- `@chat:reference-preview-cards` — A chat message containing `[[id]]` references for issues / patches / documents / sessions / conversations renders a preview card per unique referenced object at the end of the message, in source order, deduplicated. Inline `[[id]]` rendering in the message body is unchanged.
- `@chat:proxy-tab` — The right-panel Proxy tab is hidden when the conversation's active session has no advertised `proxy_targets`. Once a worker advertises a port (via `POST /v1/sessions/<sid>/proxy-targets`), the tab appears with a per-port row and a status badge driven by `useConversationProxyStatus` (HEAD probe against `<port>-<conv-id>.proxy.<host>`). Clicking "Open in new tab" calls `POST /v1/conversations/<cid>/proxy-auth` to mint the proxy cookie, then `window.open`s the proxy URL — never iframed.

## Repositories

- `@repos:edit-merge-policy` — User can view, edit, clear, and round-trip a repository's `merge_policy` via the Repository edit modal's JSON textarea, with inline error on invalid JSON

## Triggers

- `@triggers:create-form` — The create-trigger modal's Status picker is disabled until a Project is picked; picking a project enables the Status picker and lists that project's statuses; changing the project clears the previously-selected status and re-derives the list; Add Trigger stays disabled until both fields are set; submitting POSTs a `CreateIssueAction` whose `project_id` + `status` reflect the user's picks.

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
- `@mobile:list-overflow` — List pages (sessions, patches, issues, chats, repositories, agents, secrets) have no document-level horizontal overflow at 360, 375, and 400 px.
- `@mobile:related-tab-overflow` — Detail-page Related/Activity/Details tabs on issue and chat detail fit the mobile viewport with no element extending past the right edge at 360, 375, and 400 px.
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
