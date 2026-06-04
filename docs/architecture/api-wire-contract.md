# API wire contract

[`hydra-common::api::v1`](../../hydra-common/src/api/v1/) is the **wire contract** between the server, the CLI, and the web frontend. The Rust types in that module are the contract; treat them as such.

## What lives here

`hydra-common/src/api/v1/` holds request/response structs, query parameter types, and the public enums (e.g. `IssueStatus`, `Status`, `PatchKind`) — everything that crosses the HTTP boundary. The `#[non_exhaustive]` attribute and `#[serde(other)]` `Unknown` fallback (see `IssueStatus`) are part of the contract: they let new variants ship without breaking older clients.

## Evolution rule: additive only

Changes to `hydra-common::api::v1::*` must be **additive**.

```rust
// correct: add a new optional field
#[derive(Serialize, Deserialize)]
pub struct UpsertIssueRequest {
    pub issue: Issue,
    pub session_id: Option<SessionId>,
    pub label_ids: Vec<LabelId>,
    pub label_names: Vec<String>,
    pub initial_actor: Option<ActorRef>, // new — defaults via Option/Default
}
```

```rust
// wrong: rename or repurpose an existing field
#[derive(Serialize, Deserialize)]
pub struct UpsertIssueRequest {
    pub issue: Issue,
    pub task_id: Option<SessionId>, // was `session_id` — breaks every existing client
    ...
}
```

Allowed: new fields (use `Option<T>` or `#[serde(default)]`), new enum variants on `#[non_exhaustive]` enums, new request/response types. Not allowed: renames, removals, type changes, tightening required fields, changing wire tag literals (the `kebab-case` discriminator strings on `IssueStatus` and friends are part of the contract).

## When you change an API type, do all of this

1. Update the Rust type in `hydra-common/src/api/v1/<entity>.rs`.
2. Update the corresponding `domain::<entity>` type in `hydra-server/src/domain/<entity>.rs` and its `From` conversion impls in both directions. The store and policy engine work in `domain`, so the conversion must be exhaustive — if the new variant is added without updating the `From` impl, the `unreachable!` in the `From<api::…>` arm (e.g. [`hydra-server/src/domain/issues.rs:308,334,357`](../../hydra-server/src/domain/issues.rs)) will panic at runtime the first time the route handler converts an incoming request.
3. Regenerate the TypeScript bindings:

   ```
   cd hydra-web && pnpm generate-types
   ```

   Under the hood this runs `TS_RS_EXPORT_DIR=hydra-web/packages/api/src/generated cargo test -p hydra-common --features ts export_bindings` (see [`hydra-common/src/lib.rs`](../../hydra-common/src/lib.rs)) and prettier-formats the output.

4. Verify the frontend still compiles: `cd hydra-web && pnpm typecheck`.
5. Add a wire-format shape test in `hydra-common` if the change introduces a new tag or representation — the JSON literals are *our* contract, not serde's.

## Conventions worth knowing

- Wire enums use `#[serde(rename_all = "kebab-case")]`. The string `"in-progress"` is part of the API; don't change the casing.
- `#[non_exhaustive]` on a wire enum with a `Unknown` `#[serde(other)]` variant lets older clients accept new variants gracefully. Add new variants this way too.
- All IDs use the `HydraId`-backed newtypes (`IssueId`, `SessionId`, …). Routes accept and emit those types; the store generates them — see [`domain-store-routes.md`](./domain-store-routes.md).
- Query-param structs implement `Serialize`/`Deserialize` and rely on `serde_urlencoded`; helper functions for principal-path encoding (e.g. on `SearchIssuesQuery`) keep query strings stable across URL escaping. Don't bypass them.

## CI guard

`.github/workflows/ci.yml` runs the ts-rs export step. If you forget to regenerate, the generated files drift from the Rust source and CI will fail on the diff.
