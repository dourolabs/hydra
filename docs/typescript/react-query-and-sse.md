# React Query and SSE

Server state lives in React Query v5. The SSE stream from the BFF (`/api/v1/events`) feeds entity mutations directly into the query cache — so the SPA stays live without polling.

## React Query hooks

Each feature owns a `use<Entity>.ts` next to its component. Hook bodies are small and follow the same shape:

```ts
// useIssue.ts
import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

export function useIssue(issueId: string) {
  return useQuery({
    queryKey: ["issue", issueId],
    queryFn: () => apiClient.getIssue(issueId),
    enabled: !!issueId,
  });
}
```

Three things to keep consistent:

- **`queryKey`** — tuple starting with the entity name, then identifying inputs. `["issue", id]`, `["issues", "batch", idsParam]`, `["paginatedIssues", filters]`. The SSE handler relies on these prefixes to invalidate the right caches; new keys should follow the same shape.
- **`queryFn`** — always calls a method on `apiClient`. No inline `fetch` (see [packages.md](./packages.md)).
- **`enabled`** — guard on inputs being defined so the hook is safe to call unconditionally from the component.

Before adding a hook, check the relevant `features/<name>/` directory for an existing one to extend.

## SSE → cache, not polling

`packages/web/src/hooks/useSSE.ts` opens one EventSource at `/api/v1/events` for the whole app and translates server events into cache updates:

- For `*_created` / `*_updated` events that carry the new entity body, it upserts directly into list caches (`["issues"]`, `["sessions"]`, batch keys, etc.) and invalidates the matching `["entity", id]` detail key.
- For `*_deleted` events it removes from list caches and drops the detail key.
- For page-level keys (`["paginatedIssues"]`, `["issueCount"]`, `["chatRelated"]`, …) it issues targeted invalidations.
- On reconnect, visibility change, or `resync` events it runs `invalidatePageAndTreeCaches` — a debounced refetch of just the page/tree-level keys, not every query in the cache.

```tsx
// wrong — manual polling defeats the SSE pipeline
useQuery({ queryKey: ["issue", id], queryFn: …, refetchInterval: 2000 });

// correct — let useSSE keep the cache fresh
useQuery({ queryKey: ["issue", id], queryFn: () => apiClient.getIssue(id), enabled: !!id });
```

`useSSE` is mounted once in `packages/web/src/layout/AppLayout.tsx`; component code should not open its own EventSource. Session log chunks are multiplexed through the same connection via `sessionLogRegistry`.

## Feature module shape

```
features/<name>/
  <Component>.tsx          ← consumer
  <Component>.module.css   ← scoped styles
  use<Entity>.ts           ← React Query hook
```

Hooks and components live next to each other but in separate files — see the HMR rule in [style.md](./style.md).

## See also

- [packages.md](./packages.md) — `HydraApiClient` and the `utils/` checklist.
- [testing.md](./testing.md) — how to exercise SSE behaviour in e2e.
