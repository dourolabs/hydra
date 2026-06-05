# Idioms

Design-level patterns that come up when modelling state, building APIs, or
wiring background work. For lower-level syntactic conventions see
[style.md](style.md).

## No placeholder or sentinel values

Don't use `"unknown"`, `""`, `-1`, or similar stand-ins for "we don't have a
value yet." Demand the real value at construction time, or model the absence
explicitly with `Option`/an enum variant. Sentinels survive serialization and
spread silently through downstream code.

```rust
// wrong
struct Issue { assignee: String } // "" means unassigned

// correct
struct Issue { assignee: Option<Principal> }
```

**Wire-type carve-out.** If the field lives on a `#[derive(Serialize,
Deserialize)]` wire type in `hydra-common/src/api/v1/**` and the type already
ships, you cannot retroactively change its on-wire shape — this would break
existing consumers and stored JSON. See
[`docs/architecture/api-wire-contract.md`](../architecture/api-wire-contract.md)
("changes must be additive"). The sentinel-to-`Option<T>` rule applies only
to:

- new wire types being introduced for the first time, and
- non-wire types (`domain::*` types, internal helpers, store records).

For existing wire types with empty-string sentinels (e.g.
`AgentRecord.prompt_path`, `Issue.title`, `Issue.progress`), the sentinel is
part of the API contract and stays.

## The store owns id generation

Entity ids are minted by the store layer. Callers — routes, jobs, CLI
commands — never construct or pass in an id when creating a new entity.
This keeps the id space monotonic and lets the store enforce uniqueness in
one place.

```rust
// wrong
store.create_issue(IssueId::new("i-abc123"), payload)?;

// correct
let id = store.create_issue(payload)?; // store returns the new id
```

## Mandatory fields over `Option` with defaults

Prefer required fields to optional fields with a synthetic default. A
`Default` impl on a domain type tends to encode an invalid state ("a blank
issue"). If a field truly is optional, use `Option<T>`; if it's required, make
the constructor demand it.

Wire-format compatibility on API v1 types is a separate concern — additions
there must stay backward-compatible (see `hydra-common/AGENTS.md`).

## Constructor parameters, not builders

Prefer `Foo::new(a, b, c)` to `Foo::builder().with_a(a).with_b(b).build()`.
Builders earn their keep when there are many genuinely-optional fields with
non-trivial defaults; for the common case where the caller has all the values,
a plain constructor is shorter and impossible to misuse.

```rust
// wrong
let req = UpsertIssueRequest::builder()
    .with_title("...")
    .with_description("...")
    .build();

// correct
let req = UpsertIssueRequest::new(title, description);
```

## Secrets travel through env vars, not API types

API tokens, credentials, and other secrets pass to workers and child
processes via environment variables (e.g. `OPENAI_API_KEY`). They do not
belong on `WorkerContext`, request/response types, or stored entities. This
keeps secrets out of logs, the database, and the wire format.

## Be careful with `serde(flatten)`

`#[serde(flatten)]` is occasionally the right tool (e.g. composing a config
struct from sub-structs), but it makes the JSON shape implicit and breaks
when two flattened structs share a field name. Prefer a named, nested field
unless you have a specific reason for flattening.

```rust
// usually wrong on API types
struct Outer {
    foo: String,
    #[serde(flatten)]
    inner: Inner,
}

// correct
struct Outer {
    foo: String,
    inner: Inner,
}
```
