# hydra-web Agent Guidelines

This document helps AI agents and developers navigate the hydra-web frontend codebase. Read this before making changes to avoid duplicating existing utilities or diverging from established patterns.

## Project Structure Overview

Three-package monorepo managed with pnpm workspaces:

| Package | Name | Purpose |
|---------|------|---------|
| `packages/api` | `@hydra/api` | Typed API client and auto-generated types |
| `packages/ui` | `@hydra/ui` | Reusable component library |
| `packages/web` | `@hydra/web` | React SPA + Hono BFF |

**Dependency order:** `api` → `ui` → `web`. Changes to `api` or `ui` require rebuilding downstream packages.

## Shared Utility Functions (`packages/web/src/utils/`)

Always check these modules before writing new helpers:

- **`statusMapping.ts`** — `normalizeIssueStatus()`, `normalizeSessionStatus()`, `normalizePatchStatus()`, `normalizeCiState()` for normalizing API entity statuses to `BadgeStatus` values. Each normalizer converts domain-specific status strings (e.g., PascalCase patch statuses) to kebab-case Badge variants without lossy mapping.
- **`time.ts`** — `formatDuration()`, `getRuntime()` for time formatting. All time-related formatting should go here.
- **`text.ts`** — `descriptionSnippet()` for truncating descriptions to preview length.
- **`actors.ts`** _(planned)_ — Will contain `actorDisplayName()`, `actorAvatarName()` for rendering actor references. Until created, check for inline actor formatting logic and consolidate here.

## Feature Module Pattern

Each feature in `packages/web/src/features/` follows the same structure:

```
features/<name>/
  ├── <Component>.tsx          # React component
  ├── <Component>.module.css   # CSS Module styles
  └── use<Entity>.ts           # React Query hook
```

Current feature modules: `auth`, `issues`, `sessions`, `patches`.

**Hook pattern** — All data-fetching hooks wrap React Query:
```ts
useQuery({
  queryKey: ["entity", id],
  queryFn: () => apiClient.method(id),
  enabled: !!id,
})
```

Before creating a new hook, check existing hooks in the relevant feature module.

## UI Component Library (`@hydra/ui`)

14 reusable components exported from `packages/ui/src/index.ts`:

`Avatar`, `Badge`, `Button`, `Input`, `LogViewer`, `MarkdownViewer`, `Modal`, `Panel`, `Select`, `Spinner`, `Tabs`, `Textarea`, `Tooltip`, `TreeView`

**Always use these components** rather than building inline equivalents. Each component lives in `packages/ui/src/components/<Name>/` with its own `.tsx` and `.module.css`.

Theme tokens are defined in `packages/ui/src/theme/tokens.css`.

## API Client (`@hydra/api`)

- **Auto-generated types** from Rust via ts-rs in `packages/api/src/generated/`.
- **Client class**: `HydraApiClient` in `packages/api/src/client.ts`.
- **Singleton instance**: `apiClient` in `packages/web/src/api/client.ts`.
- **Never create direct `fetch` calls** — add methods to `HydraApiClient` instead.

## Cross-Workspace Build Verification
When Rust API types in `hydra-common` change, TypeScript types must be regenerated. Run `pnpm typecheck` from the `hydra-web/` directory to verify the frontend still compiles against the updated types.

## Build / Dev Commands

Run from the `hydra-web/` directory:

| Command | Purpose |
|---------|---------|
| `pnpm build` | Build the React SPA |
| `pnpm typecheck` | TypeScript checking across all packages |
| `pnpm lint` | Lint all packages |
| `pnpm -r build` | Build all packages (respects dependency order) |

## Key Conventions

- **CSS Modules** for all styling (`.module.css`). No global CSS or inline styles.
- **React Router v7** for routing — routes defined in `packages/web/src/router.tsx`.
- **React Query v5** for server state management.
- **SSE** via `useSSE()` hook (`packages/web/src/hooks/useSSE.ts`) for real-time entity updates. Automatically invalidates React Query caches on server events.
- **Dark terminal theme** — black background, green accent. Respect existing theme tokens.
- **Check `utils/`** before writing new utility functions to avoid duplication.
- **Do not export hooks and components from the same file.** Mixing component exports and hook exports in a single module breaks React Fast Refresh (HMR). Place hooks in their own `use<Name>.ts` file next to the component.

## Testing Frontend Changes

Before submitting a patch, verify your changes using the dev testing stack.

### Quick start

1. Install dependencies: `cd hydra-web && pnpm install`
2. Install Playwright browsers (not needed in the worker Docker image): `pnpm --filter @hydra/web exec playwright install chromium`
3. Run E2E tests: `pnpm e2e`
   - Playwright automatically starts the mock server, BFF, and frontend via its `webServer` config
   - Servers are automatically stopped when tests complete
4. If tests fail, check screenshots in `packages/web/test-results/`
5. If tests pass, create your patch

> **WARNING: Do not use `dev-test.sh --test` with `run_in_background`.** Background dev servers can outlive the agent session and cause job hangs. Always use `pnpm e2e` instead — Playwright manages the full server lifecycle automatically.

> **Do not start dev servers manually before running `pnpm e2e`.** Playwright's `reuseExistingServer: true` means it will skip starting servers if the ports are already occupied, but manually started servers won't be cleaned up when tests finish. Let Playwright handle it.

Use `./scripts/dev-test.sh` only for interactive development where you need long-running servers (e.g., manual browser testing). Never use it in automated or agent workflows.

### Ports

| Service | Port | Purpose |
|---------|------|---------|
| Mock server | 8080 | Standalone TypeScript mock of the hydra API |
| BFF | 4000 | Hono backend-for-frontend (proxies to mock server) |
| Frontend | 3000 | Vite React dev server |

### Reset mock server state

`POST http://localhost:8080/v1/dev/reset` reloads seed data. Use this between tests to restore a clean state.

### Simulate server errors

Add the `X-Mock-Error: <status-code>` header to any request to make the mock server return that HTTP status. This is useful for testing error handling in the frontend.

### Run specific tests

```bash
pnpm e2e                                           # all E2E tests
pnpm --filter @hydra/web exec playwright test login # specific test file
pnpm --filter @hydra/web exec playwright test --headed  # visible browser
```

### Debugging test failures

- Screenshots are saved to `packages/web/test-results/` on failure.
- Traces are recorded on first retry (CI only by default). View with `pnpm --filter @hydra/web exec playwright show-trace <trace-file>`.
- Run with `--headed` to watch the browser during test execution.
- Playwright's `webServer` config in `packages/web/playwright.config.ts` auto-starts all three servers (mock server, BFF, frontend) when running `pnpm e2e`.

### Visual Audit & Screenshot Capture

The visual audit script captures screenshots of every major page at both desktop (1280×720) and mobile (375×812) viewports. Use it before and after making CSS or layout changes to catch visual regressions.

#### Running the visual audit

1. Start the dev stack for interactive use: `./scripts/dev-test.sh`
2. Run the visual audit: `cd packages/web && pnpm visual-audit`
3. Screenshots are saved to `packages/web/test-results/visual-audit/`
4. Stop the dev stack when done (Ctrl-C the `dev-test.sh` process)

Each screenshot is named `{viewport}-{page}.png`, for example:
- `desktop-dashboard.png`, `mobile-dashboard.png`
- `desktop-issue-detail.png`, `mobile-issue-detail.png`

#### When to run

- **Before** making CSS, layout, or component changes — to establish a baseline
- **After** making changes — to verify nothing regressed
- Compare before/after screenshots side-by-side to spot unintended differences

#### Pages captured

Login, dashboard, issue detail, patch detail, documents list, document detail, settings, and session log page.

### Contract tests

The `@hydra/mock-server` package includes contract tests that validate the mock server's responses against the `@hydra/api` client types. These run as part of `pnpm test` in CI and catch drift between the mock and real server.
