# TypeScript reference

Reference docs for the `hydra-web/` workspace — three packages, React 19 + Vite SPA, dark terminal theme, server state via React Query v5 plus an SSE stream that mutates the cache in place.

Workspace shape — build in dependency order:

```
packages/api   →  @hydra/api   typed client + generated types
packages/ui    →  @hydra/ui    component library + theme tokens
packages/web   →  @hydra/web   the SPA itself
```

`packages/mock-server` (`@hydra/mock-server`) sits alongside but isn't a dependency of `web`; it backs e2e and contract tests.

## Docs

- [packages.md](./packages.md) — workspace layout, `HydraApiClient` as the single API entry point, `utils/` and `@hydra/ui` checklists before adding new code.
- [style.md](./style.md) — CSS Modules only, dark terminal theme tokens, and the HMR rule against co-exporting hooks and components.
- [react-query-and-sse.md](./react-query-and-sse.md) — React Query v5 query-key conventions and how `useSSE()` keeps the cache live without polling.
- [testing.md](./testing.md) — `pnpm e2e`, mock-server reset and error-injection, visual-audit, and contract tests.
