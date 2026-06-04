# Domain / store / routes layering

Server-side hydra is split into three layers. Each one has one job; mixing concerns across them is the most common review comment.

## The split

| Layer | Crate path | What goes here |
|---|---|---|
| `routes/` | [`hydra-server/src/routes/`](../../hydra-server/src/routes/) | HTTP/Axum handlers. Translate `hydra_common::api::v1::*` ↔ `domain::*`, call `AppState`, map errors to `ApiError`. No business rules. |
| `domain/` | [`hydra-server/src/domain/`](../../hydra-server/src/domain/) | Server-side business types (`Issue`, `Patch`, `Session`, …) plus `From` conversions to/from the wire types. Owns server-only fields that don't belong on the wire. |
| `store/` | [`hydra-server/src/store/`](../../hydra-server/src/store/) | `Store` / `ReadOnlyStore` traits and their SQLite + in-memory implementations. Persistence and indexing only. |
| `app/` | [`hydra-server/src/app/`](../../hydra-server/src/app/) | `AppState` — the coordination layer. Composes store + policy engine + event bus; holds lifecycle validation. |

A request flows `routes → app → policy_engine → store`, then back up as wire types.

## Lifecycle validation lives in `AppState`, not the store

The store persists rows; it does not decide whether a transition is legal. Lifecycle and authorization checks run in [`AppState`](../../hydra-server/src/app/app_state.rs) by way of the [`PolicyEngine`](../../hydra-server/src/policy/mod.rs).

```rust
// correct: route → AppState → policy_engine → store
self.policy_engine
    .check_update_issue(&id, &updated_issue, None, store, &actor)
    .await?;
self.store.update_issue_with_actor(&id, updated_issue, actor).await
```

```rust
// wrong: lifecycle check inside the store impl
impl Store for SqliteStore {
    async fn update_issue(&self, id: &IssueId, issue: Issue) -> Result<...> {
        if issue.status == IssueStatus::Closed && self.has_open_children(id).await? {
            return Err(...); // policy belongs in PolicyEngine, not here
        }
        ...
    }
}
```

Why: the store is also used by tests, automations, and read-only paths that must not re-run validation. Policies are configurable (operators can swap restrictions in `hydra-server/config.yaml`), and they need read access to other entities — both incompatible with embedding rules in the persistence layer.

Stores enforce **referential** integrity only — e.g. `StoreError::InvalidDependency` for an unknown `blocked-on` target — because that protects the schema. Anything that depends on workflow state (closing with open children, illegal status transitions, branch-name collisions) goes in a [`Restriction`](../../hydra-server/src/policy/restrictions/).

## Conversions: wire ↔ domain

Every entity has paired `From` impls in `hydra-server/src/domain/<entity>.rs`:

```rust
impl From<api::issues::IssueStatus> for IssueStatus { ... }
impl From<IssueStatus> for api::issues::IssueStatus { ... }
```

Route handlers do the translation explicitly, so the store and policy engine only ever see `domain::*` types. When `hydra-common::api::v1` gains a new field or variant, the matching `domain` type and `From` impls update in the same change — see [`api-wire-contract.md`](./api-wire-contract.md).

## Reading vs. writing

- Use `&dyn ReadOnlyStore` whenever you only need to read. `Restriction` and `Automation` contexts both take `ReadOnlyStore` so policies cannot mutate state during evaluation.
- Use `&dyn Store` (extends `ReadOnlyStore`) for the write path. Mutations go through `AppState` so the event bus, policy engine, and any cascading automations see them.
- Never instantiate a store directly from a route handler; everything resolves through `AppState`.
