# Packages

`hydra-web/` is a pnpm workspace with four packages. Build order is enforced by dependency direction — `api` → `ui` → `web`. `mock-server` is a sibling, not an upstream of `web`.

| Package | Name | Role |
|---|---|---|
| `packages/api` | `@hydra/api` | `HydraApiClient`, generated request/response types, SSE primitives |
| `packages/ui` | `@hydra/ui` | Reusable components + theme tokens |
| `packages/web` | `@hydra/web` | The SPA |
| `packages/mock-server` | `@hydra/mock-server` | Hono mock of the hydra API for e2e and contract tests |

When `api` or `ui` changes, rebuild them before re-running the web typecheck:

```bash
pnpm -r build         # respects dependency order
pnpm typecheck        # tsc across all packages
```

## Never write a direct `fetch`

All hydra API calls go through `HydraApiClient` (`packages/api/src/client.ts`), held as a singleton in `packages/web/src/api/client.ts`:

```ts
// wrong — bypasses generated types, error normalization, base URL config
const r = await fetch("/api/v1/issues");

// correct
import { apiClient } from "../../api/client";
const resp = await apiClient.searchIssues({ ... });
```

If the route you need doesn't exist on `HydraApiClient`, add a method there. The one BFF-only escape hatch is `apiFetch<T>()` in the same file, used for non-hydra-server routes like `/auth/*`.

## Check `packages/web/src/utils/` before adding a helper

These already exist — extend them, don't fork them:

| Module | What's there |
|---|---|
| `statusMapping.ts` | `normalizeIssueStatus`, `normalizeSessionStatus`, `normalizePatchStatus`, `normalizeCiState` → `BadgeStatus` |
| `time.ts` | `formatDuration`, `getRuntime` — all time formatting belongs here |
| `text.ts` | `descriptionSnippet` and similar string trimming |
| `actors.ts` | `actorDisplayName`, `actorAvatarName` for actor refs |
| `sessionMapping.ts` | session-related normalizers |
| `conversationOrder.ts` | conversation ordering helpers |
| `tokens.ts` | id/token helpers |

## Check `@hydra/ui` before building inline

Exported components — full list in `packages/ui/src/index.ts`:

`Avatar`, `Badge`, `Button`, `Chip`, `CopyButton`, `DiffViewer`, `ErrorBoundary`, `HydraMark`, `Icons`, `Input`, `Kbd`, `LogViewer`, `MarkdownViewer`, `Modal`, `Panel`, `Picker`, `PreviewCard`, `Select`, `SessionStatusIndicator`, `Spinner`, `Tabs`, `Textarea`, `Toast`, `Tooltip`, `TreeView`, `TypeChip`

Search this list before writing a new pill, popover, or dialog by hand — most of the surface area is covered.

## See also

- [style.md](./style.md) — theme tokens, CSS Modules, HMR rule.
- [react-query-and-sse.md](./react-query-and-sse.md) — how data flows from `apiClient` into components.
