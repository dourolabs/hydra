# Packages

`hydra-web/` is a pnpm workspace with four packages. Build order is enforced by dependency direction — `api` → `ui` → `web`. `mock-server` is a sibling, not an upstream of `web`.

| Package | Name | Role |
|---|---|---|
| `packages/api` | `@hydra/api` | `HydraApiClient`, generated request/response types, SSE primitives |
| `packages/ui` | `@hydra/ui` | Reusable components + theme tokens |
| `packages/web` | `@hydra/web` | The SPA |
| `packages/mock-server` | `@hydra/mock-server` | Hono mock of the hydra API for integration and contract tests |

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

Non-hydra-server network probes (cross-origin reachability checks against user-supplied URLs, DNS lookups, etc.) are not subject to this rule — they aren't API calls. Keep them rare and well-commented; if you find yourself adding more than one, factor them through a named helper rather than scattering `fetch(` calls.

## Check `packages/web/src/utils/` before adding a helper

These already exist — extend them, don't fork them:

| Module | What's there |
|---|---|
| `badgeStatus.ts` | `normalizeSessionStatus`, `normalizePatchStatus`, `normalizeCiState` → `BadgeStatus`. Issue statuses are project-scoped — render via `features/projects/StatusChip` against `issue.status` (a `StatusDefinition`) instead of normalizing the key string. |
| `time.ts` | `formatDuration`, `getRuntime` — all time formatting belongs here |
| `text.ts` | `descriptionSnippet` and similar string trimming |
| `actors.ts` | `actorDisplayName`, `actorAvatarName` for `ActorRef` values (typed actor references from API responses). Note: `api/auth.ts` exports a separate `actorDisplayName` for the `ActorIdentity` auth-session shape — these are intentionally distinct surfaces, not duplicates; do not consolidate. |
| `sessionMapping.ts` | session-related normalizers |
| `conversationOrder.ts` | conversation ordering helpers |
| `tokens.ts` | id/token helpers |

## No inline components

Two rules, in order:

1. Generic components **MUST** live in `@hydra/ui` — pills, popovers, dialogs, and the like are never written inline.

   A pattern that has been re-implemented inline in three or more unrelated places is itself a generic candidate — extract to `@hydra/ui` (or `packages/web/src/components/` if cross-feature but app-specific). When extracting, preserve each call-site's existing visual output until a follow-up redesign — pattern consolidation alone shouldn't drive-by visual changes.
2. App-specific components **MUST** be extracted into `packages/web/src/features/<name>/` (or `packages/web/src/components/` if cross-feature) — not defined inside a page file.

Existing generic components — full list in `packages/ui/src/index.ts`; new generic components join this list:

`Avatar`, `Badge`, `Button`, `Chip`, `CopyButton`, `DiffViewer`, `ErrorBoundary`, `FlowPill`, `HydraMark`, `Icons`, `Input`, `Kbd`, `LogViewer`, `MarkdownViewer`, `Modal`, `Panel`, `Picker`, `PreviewCard`, `Select`, `SessionStatusIndicator`, `Spinner`, `StatusDot`, `Tabs`, `Textarea`, `Toast`, `Tooltip`, `TreeView`, `TypeChip`

App-specific extraction:

```tsx
// wrong — component defined inline in pages/FooPage.tsx
function FooHeader({ foo }: { foo: Foo }) { /* ... */ }
export function FooPage() { return <FooHeader foo={foo} />; }

// correct — extracted to features/foo/FooHeader.tsx
import { FooHeader } from "../features/foo/FooHeader";
export function FooPage() { return <FooHeader foo={foo} />; }
```

## Parameters with multiple value spaces must be split

When you're adding a query param, a function argument, or a request-body field
that could accept "either form A or form B" (e.g. a URL token that's either a
canonical id or a human-friendly key/slug), split it into two named
parameters with non-overlapping value spaces. Disambiguation by string-prefix
or content sniffing is the anti-pattern — see
["Parameter forms must be mutually exclusive by construction"](../architecture/api-wire-contract.md#parameter-forms-must-be-mutually-exclusive-by-construction)
for the rule and the silent-failure mode it prevents.

`HydraApiClient` already follows this — request types carry distinct named
fields for distinct value spaces. URL/query strings on the SPA side (e.g. the
Issues-list filter URL in `features/issues/filterUrlSync.ts`) follow the same
rule: a single `?project=` param accepts only `j-`-prefixed ids; a separate
`?project_key=` carries the slug form and is resolved to an id before any
server call.

## See also

- [style.md](./style.md) — theme tokens, CSS Modules, HMR rule.
- [react-query-and-sse.md](./react-query-and-sse.md) — how data flows from `apiClient` into components.
