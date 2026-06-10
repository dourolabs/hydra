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
        let resolved = self.resolve_status(&issue).await?;
        if resolved.unblocks_dependents && self.has_open_children(id).await? {
            return Err(...); // policy belongs in PolicyEngine, not here
        }
        ...
    }
}
```

Why: the store is also used by tests, automations, and read-only paths that must not re-run validation. Policies are configurable (operators can swap restrictions in `hydra-server/config.yaml`), and they need read access to other entities — both incompatible with embedding rules in the persistence layer.

Stores enforce **referential** integrity only — e.g. `StoreError::InvalidDependency` for an unknown `blocked-on` target — because that protects the schema. Anything that depends on workflow state (closing with open children, illegal status transitions, branch-name collisions, singleton-uniqueness flags on entity tables) goes in a [`Restriction`](../../hydra-server/src/policy/restrictions/).

The rule of thumb: *per-row* invariants (FK presence, NOT NULL, basic shape) stay on the DB / Store layer because they protect the schema and apply uniformly to every writer. *Cross-row* invariants — anything that has to look at sibling rows to decide whether this write is legal — climb up to a `Restriction` so the rule lives in one place and policies can read other entities during evaluation.

**Singleton-uniqueness flags on entity tables.** When an entity table has a boolean flag enforcing at-most-one-row-with-this-flag (e.g. `agents.is_default_conversation_agent`), enforce uniqueness through a `Restriction` called from `AppState::create_X` / `update_X`, not through partial unique indexes on the database or inline store-level checks. This is the same shape as branch-name collisions — a workflow-level cross-row check, not referential integrity. See [`AgentRoleUniquenessRestriction`](../../hydra-server/src/policy/restrictions/agent_role_uniqueness.rs) for the canonical pattern.

## Conversions: wire ↔ domain

Every entity has paired `From` impls in `hydra-server/src/domain/<entity>.rs`:

```rust
impl From<api::issues::IssueType> for IssueType { ... }
impl From<IssueType> for api::issues::IssueType { ... }
```

Route handlers do the translation explicitly, so the store and policy engine only ever see `domain::*` types. When `hydra-common::api::v1` gains a new field or variant, the matching `domain` type and `From` impls update in the same change — see [`api-wire-contract.md`](./api-wire-contract.md).

`StatusKey` is special: it is a transparent string newtype declared once in `hydra-common::api::v1::projects` and reused unchanged on both the wire and the domain side, so there is no enum-to-enum `From` to maintain. Status semantics (terminal? unblocking?) come from resolving the key against the issue's project via `AppState::resolve_status`, not from variant-matching.

## Reading vs. writing

- Use `&dyn ReadOnlyStore` whenever you only need to read. `Restriction` and `Automation` contexts both take `ReadOnlyStore` so policies cannot mutate state during evaluation.
- Use `&dyn Store` (extends `ReadOnlyStore`) for the write path. Mutations go through `AppState` so the event bus, policy engine, and any cascading automations see them.
- Never instantiate a store directly from a route handler; everything resolves through `AppState`.
