# Architecture reference

Short reference docs for the load-bearing rules that shape how hydra is built. One topic per file; read the one you need.

- [issues-and-graph.md](./issues-and-graph.md) — Issue statuses, the inferred `Ready` predicate, cascade rules, and the parent/child spawn mutex.
- [sessions-and-git.md](./sessions-and-git.md) — Per-session tracking branches, the bundle mount `setup` / `save` phases, and how sequential agents pick up prior work.
- [domain-store-routes.md](./domain-store-routes.md) — The `routes/` ↔ `domain/` ↔ `store/` three-layer split and why lifecycle validation lives in `AppState`, not the store.
- [automations-vs-background-workers.md](./automations-vs-background-workers.md) — When to write an [`Automation`](../../hydra-server/src/policy/mod.rs) vs. a `ScheduledWorker`, and how each is wired into the engine.
- [api-wire-contract.md](./api-wire-contract.md) — `hydra-common::api::v1` is the wire contract; how to evolve it additively and what needs to follow when types change.
