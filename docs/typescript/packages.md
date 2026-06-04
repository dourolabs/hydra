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

If the route you need doesn't exist on `HydraApiClient`, add a method there. The one BFF-only escape hatch is `apiFetch<T>()` in `packages/web/src/api/client.ts` (alongside the `apiClient` singleton), used for non-hydra-server routes like `/auth/*`.

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

## No inline components

Two rules, in order:

1. Generic components **MUST** live in `@hydra/ui` — pills, popovers, dialogs, and the like are never written inline.
2. App-specific components **MUST** be extracted into `packages/web/src/features/<name>/` (or `packages/web/src/components/` if cross-feature) — not defined inside a page file.

Existing generic components — full list in `packages/ui/src/index.ts`; new generic components join this list:

`Avatar`, `Badge`, `Button`, `Chip`, `CopyButton`, `DiffViewer`, `ErrorBoundary`, `HydraMark`, `Icons`, `Input`, `Kbd`, `LogViewer`, `MarkdownViewer`, `Modal`, `Panel`, `Picker`, `PreviewCard`, `Select`, `SessionStatusIndicator`, `Spinner`, `Tabs`, `Textarea`, `Toast`, `Tooltip`, `TreeView`, `TypeChip`

App-specific extraction:

```tsx
// wrong — component defined inline in pages/FooPage.tsx
function FooHeader({ foo }: { foo: Foo }) { /* ... */ }
export function FooPage() { return <FooHeader foo={foo} />; }

// correct — extracted to features/foo/FooHeader.tsx
import { FooHeader } from "../features/foo/FooHeader";
export function FooPage() { return <FooHeader foo={foo} />; }
```

## See also

- [style.md](./style.md) — theme tokens, CSS Modules, HMR rule.
- [react-query-and-sse.md](./react-query-and-sse.md) — how data flows from `apiClient` into components.
