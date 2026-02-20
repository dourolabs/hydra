# metis-web Agent Guidelines

This document helps AI agents and developers navigate the metis-web frontend codebase. Read this before making changes to avoid duplicating existing utilities or diverging from established patterns.

## Project Structure Overview

Three-package monorepo managed with pnpm workspaces:

| Package | Name | Purpose |
|---------|------|---------|
| `packages/api` | `@metis/api` | Typed API client and auto-generated types |
| `packages/ui` | `@metis/ui` | Reusable component library |
| `packages/web` | `@metis/web` | React SPA + Hono BFF |

**Dependency order:** `api` → `ui` → `web`. Changes to `api` or `ui` require rebuilding downstream packages.

## Shared Utility Functions (`packages/web/src/utils/`)

Always check these modules before writing new helpers:

- **`statusMapping.ts`** — `issueToBadgeStatus()`, `jobToBadgeStatus()`, `patchToBadgeStatus()` for converting entity statuses to `Badge` component variants. Add new status mappers here (e.g., CI state badges).
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

Current feature modules: `auth`, `issues`, `jobs`, `patches`.

**Hook pattern** — All data-fetching hooks wrap React Query:
```ts
useQuery({
  queryKey: ["entity", id],
  queryFn: () => apiClient.method(id),
  enabled: !!id,
})
```

Before creating a new hook, check existing hooks in the relevant feature module.

## UI Component Library (`@metis/ui`)

14 reusable components exported from `packages/ui/src/index.ts`:

`Avatar`, `Badge`, `Button`, `Input`, `LogViewer`, `MarkdownViewer`, `Modal`, `Panel`, `Select`, `Spinner`, `Tabs`, `Textarea`, `Tooltip`, `TreeView`

**Always use these components** rather than building inline equivalents. Each component lives in `packages/ui/src/components/<Name>/` with its own `.tsx` and `.module.css`.

Theme tokens are defined in `packages/ui/src/theme/tokens.css`.

## API Client (`@metis/api`)

- **Auto-generated types** from Rust via ts-rs in `packages/api/src/generated/`.
- **Client class**: `MetisApiClient` in `packages/api/src/client.ts`.
- **Singleton instance**: `apiClient` in `packages/web/src/api/client.ts`.
- **Never create direct `fetch` calls** — add methods to `MetisApiClient` instead.

## Build / Dev Commands

Run from the `metis-web/` directory:

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
