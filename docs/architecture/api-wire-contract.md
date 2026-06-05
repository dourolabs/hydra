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

Allowed: new fields (use `Option<T>` or `#[serde(default)]`), new enum variants on `#[non_exhaustive]` enums, new request/response types. Not allowed: renames, removals, type changes, tightening required fields, changing wire tag literals (the `kebab-case` discriminator strings on `IssueStatus` and friends are part of the contract). (See also [`docs/rust/idioms.md`](../rust/idioms.md) — the `Option<T>` sentinel rule explicitly excludes existing wire types for this reason.)

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
6. If the enum is a tagged union with payload-carrying variants, pair `#[non_exhaustive]` with the `<EnumName>Helper` + custom `Deserialize` pattern (see [`#[non_exhaustive]` on tagged-union enums](#non_exhaustive-on-tagged-union-enums) below). `#[serde(other)]` alone can't carry an `Unknown` fallback through an externally-tagged shape, and a payload-bearing variant can't be the catch-all.

## Conventions worth knowing

- **New** wire enums should use `#[serde(rename_all = "kebab-case")]` (e.g. `IssueStatus` with `"in-progress"`). **Existing** wire formats are grandfathered — never change a live wire tag literal, since that breaks every existing client. Many older multi-word enums ship with `snake_case` for that reason (e.g. `task_status::Status`, `SessionMode`, `SessionEvent`, `MountItem`, `Bundle`, `relay::*`, `merge_check::*`); leave their casing alone.
- `#[non_exhaustive]` on a wire enum with a `Unknown` `#[serde(other)]` variant lets older clients accept new variants gracefully — add new variants this way for *unit-variant* enums (`IssueStatus`, `Status`, `PatchKind`). Tagged unions whose variants carry payload need a slightly different shape; see [the dedicated section](#non_exhaustive-on-tagged-union-enums) below.
- All IDs use the `HydraId`-backed newtypes (`IssueId`, `SessionId`, …). Routes accept and emit those types; the store generates them — see [`domain-store-routes.md`](./domain-store-routes.md).
- Query-param structs implement `Serialize`/`Deserialize` and rely on `serde_urlencoded`; helper functions for principal-path encoding (e.g. on `SearchIssuesQuery`) keep query strings stable across URL escaping. Don't bypass them.

## `#[non_exhaustive]` on tagged-union enums

`#[serde(other)]` requires the catch-all variant to be a unit variant, and the enum to be internally tagged (`#[serde(tag = "…")]`). For *externally*-tagged enums and for any enum where the forward-compat story has to tolerate richer payload changes (renamed/removed fields on known variants, etc.), `#[serde(other)]` alone isn't enough — you also need a private `<EnumName>Helper` plus a hand-written `Deserialize` that converts a parse failure into the `Unknown` variant.

Canonical examples live in [`hydra-common/src/api/v1/sessions.rs`](../../hydra-common/src/api/v1/sessions.rs): see `Bundle` / `BundleHelper` (~lines 327–368) and `MountItem` / `MountItemHelper` (~lines 480–579). Sketch:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Bundle {
    None,
    GitRepository { url: String, rev: String },
    #[serde(other)]
    Unknown,
}

// Private helper mirrors the public shape but without `Unknown`. Lives in
// the same module; never re-exported.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BundleHelper {
    None,
    GitRepository { url: String, rev: String },
}

impl<'de> Deserialize<'de> for Bundle {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        match serde_json::from_value::<BundleHelper>(v) {
            Ok(BundleHelper::None) => Ok(Bundle::None),
            Ok(BundleHelper::GitRepository { url, rev }) => Ok(Bundle::GitRepository { url, rev }),
            Err(_) => Ok(Bundle::Unknown),
        }
    }
}
```

For flat internally-tagged enums whose only forward-compat concern is unknown *tags* (not unknown fields), the simpler `#[serde(other)] Unknown` shape is enough — `SessionEvent::Unknown` at `hydra-common/src/api/v1/sessions.rs:996-997` is the model.

## Safety-critical wire enums (rejection-on-unknown)

A small set of wire types are *policy boundaries*: their variants correspond to decisions about who is allowed to do what. For those, forward-compat tolerance is a footgun — silently mapping an unrecognized tag to `Unknown` could mask a policy bug or let a server send a value the client should have rejected outright. These enums get `#[non_exhaustive]` (so the Rust side stays additive) but explicitly do **not** get `#[serde(other)] Unknown`: deserialization of an unknown tag must fail.

The canonical example is [`merge_check::*`](../../hydra-common/src/api/v1/merge_check.rs) — every wire enum in that module is `#[non_exhaustive]` without an `Unknown` variant, and every one of them has a `unknown_*_is_rejected` test asserting the wire-side rejection (`unknown_code_is_rejected`, `unknown_blocked_at_layer_is_rejected`, `unknown_reason_kind_is_rejected`, etc.).

If you add a new safety-critical wire enum, follow that pattern:

1. Apply `#[non_exhaustive]` (Rust-side additive forward compat).
2. Do **not** add `#[serde(other)] Unknown`.
3. Add an inline `// safety-critical: rejection-on-unknown` comment so the next reader understands the missing `Unknown` is intentional, not an oversight.
4. Add a `unknown_<thing>_is_rejected` test that asserts `serde_json::from_str::<YourEnum>("\"made-up-tag\"")` returns `Err`.

## CI guard

`.github/workflows/ci.yml` runs the ts-rs export step. If you forget to regenerate, the generated files drift from the Rust source and CI will fail on the diff.
