# `hydra graph` query DSL — reference

This is the long-form reference for the pipe-grammar query argument shared
by `hydra graph search`, `hydra graph diff`, and `hydra graph log`. For a
short introduction, run `hydra graph search --help` (or `diff` / `log`).
For the design rationale, see `/designs/hydra-graph-query-language.md` in
the document store.

## Synopsis

A hydra graph query is a pipeline of vertex-set stages separated by `|`.
The first element produces an initial vertex set; each subsequent stage
takes the running vertex set as input and produces a new vertex set. The
terminal set is hydrated into full node records and rendered. **The
simplest valid query is a bare id.**

## Quickstart

Five examples that cover the everyday cases:

```shell
# 1. Bare id — the simplest valid query. No /v1/relations call; just
#    hydrate the one node.
hydra graph search 'i-abc123'

# 2. Scope — the canonical "this issue plus everything it owns" expansion
#    (descendants via child-of, plus attached patches and documents).
hydra graph search 'i-abc123 | scope'

# 3. Scope, filtered to patches — the typical "what patches has this
#    feature shipped?" query.
hydra graph search 'i-abc123 | scope | kind=patch'

# 4. Direct children (inclusive). The seed is preserved even when the
#    issue has no children — this is the DSL's inclusive-by-default
#    contract.
hydra graph search 'i-abc123 | children'

# 5. Direct children (exclusive). Drops the seed; equivalent to the old
#    `--source i-abc123` flag form.
hydra graph search 'i-abc123 | children exclusive'
```

## Grammar

```
QUERY        := SOURCE ('|' STAGE)*
SOURCE       := ID (',' ID)*
STAGE        := RELATION_STAGE | FILTER_STAGE
RELATION_STAGE := NAME ARG*
NAME         := 'parents' | 'children' | 'neighbors'
              | 'ancestors' | 'descendants' | 'scope'
ARG          := 'rel=' RELTYPE | 'transitive' | 'exclusive'
FILTER_STAGE := 'kind=' KINDLIST
KINDLIST     := KIND (',' KIND)*
KIND         := 'issue' | 'patch' | 'document' | 'conversation'
RELTYPE      := 'child-of' | 'blocked-on' | 'has-patch'
              | 'has-document' | 'refers-to'
ID           := /[a-z]-[a-z0-9]+/
```

Whitespace is permitted around `|` and between tokens within a stage. Arg
order within a stage is free (`children rel=child-of transitive exclusive`
parses to the same AST as `children exclusive transitive rel=child-of`).
Duplicate args within a single stage are a parse error.

### Inclusive-by-default

The five relation stages (`parents`, `children`, `neighbors`, `ancestors`,
`descendants`) default to **inclusive** semantics:

> `V | stage` = `V ∪ traversal(V)` — the input set is preserved.

The seed survives even when the traversal returns zero rows (e.g.,
`i-X | children rel=has-patch` on an issue with no patches still returns
`{i-X}`). To restore the old "exclude the seed" behavior (matching today's
`--source` / `--target` / `--object` flags), append the bare keyword
`exclusive` to the stage. The `scope` and `kind=` stages do not accept
`exclusive` (scope is inherently inclusive; kind is a filter).

## Stage catalog

### `parents [rel=R] [transitive] [exclusive]`

**Signature.** Walks edges into the input set: any edge whose target is in
`V` contributes its source.

**Semantics.** Default: `V' = V ∪ { r.source_id : r ∈ response }`.
With `exclusive`: `V' = { r.source_id : r ∈ response } \ V`.

**Args.** `rel=R` is optional. `transitive` requires `rel=` (the server
constraint, mirrored client-side); writing `transitive` without `rel=` is
a parse error pointing at the `transitive` token.

**Lowering shape.** `GET /v1/relations?target_ids=<V>[&rel_type=R][&transitive=true]`.

**Examples.**

```shell
hydra graph search 'i-x | parents'
hydra graph search 'i-x | parents rel=child-of'
hydra graph search 'i-x | parents rel=child-of transitive'
hydra graph search 'i-x | parents rel=child-of transitive exclusive'
```

### `children [rel=R] [transitive] [exclusive]`

**Signature.** Walks edges out of the input set: any edge whose source is
in `V` contributes its target.

**Semantics.** Default: `V' = V ∪ { r.target_id : r ∈ response }`.
With `exclusive`: `V' = { r.target_id : r ∈ response } \ V`.

**Args.** Same rules as `parents` — `rel=` optional, `transitive` requires
`rel=`.

**Lowering shape.** `GET /v1/relations?source_ids=<V>[&rel_type=R][&transitive=true]`.

**Examples.**

```shell
hydra graph search 'i-x | children'
hydra graph search 'i-x | children rel=child-of'
hydra graph search 'i-x | children rel=child-of transitive'
hydra graph search 'i-x | children rel=child-of transitive exclusive'
```

### `neighbors [rel=R] [exclusive]`

**Signature.** Walks any matching edge touching the input set: each edge
whose source **or** target is in `V` contributes both endpoints.

**Semantics.** Default: `V' = V ∪ T(V)` where `T(V) = { r.source_id : r }
∪ { r.target_id : r }` from the per-stage response.
With `exclusive`: `V' = T(V) \ V` (matches the old `--object` flag).

**Args.** `rel=R` is optional. `transitive` is **not** accepted — the
server cannot do transitive closure with `object_id`/`object_ids`. Use
`parents rel=R transitive` or `children rel=R transitive` if you need a
direction-typed transitive walk.

**Lowering shape.** `GET /v1/relations?object_ids=<V>[&rel_type=R]`. For a
single-element vertex set, the resolver may use the equivalent singular
form `object_id=<v>`; both are accepted by the server.

**Examples.**

```shell
hydra graph search 'i-x | neighbors'
hydra graph search 'i-x | neighbors rel=refers-to'
hydra graph search 'i-x | neighbors rel=refers-to exclusive'
```

### `ancestors rel=R [exclusive]`

**Signature.** Sugar for `parents rel=R transitive`. `rel=` is **required**
(transitive needs a rel filter). Explicit `transitive` is rejected with a
hint pointing at the canonical `parents rel=R transitive` form.

**Semantics.** Default: same as `parents rel=R transitive` (inclusive).
With `exclusive`: same as `parents rel=R transitive exclusive`.

**Lowering shape.** Lowered to `parents rel=R transitive` before reaching
the resolver, so the HTTP shape is
`GET /v1/relations?target_ids=<V>&rel_type=R&transitive=true`.

**Examples.**

```shell
hydra graph search 'i-x | ancestors rel=child-of'
hydra graph search 'i-x | ancestors rel=child-of exclusive'
```

### `descendants rel=R [exclusive]`

**Signature.** Sugar for `children rel=R transitive`. `rel=` is
**required**. Explicit `transitive` is rejected.

**Semantics.** Default: same as `children rel=R transitive` (inclusive).
With `exclusive`: same as `children rel=R transitive exclusive`.

**Lowering shape.** Lowered to `children rel=R transitive` before reaching
the resolver, so the HTTP shape is
`GET /v1/relations?source_ids=<V>&rel_type=R&transitive=true`.

**Examples.**

```shell
hydra graph search 'i-x | descendants rel=child-of'
hydra graph search 'i-x | descendants rel=child-of exclusive'
```

### `scope`

**Signature.** No args. Distributes the canonical scope expansion over
each issue in the input set: for each `v ∈ V`, expand to `{v} ∪ transitive
child-of descendants of v ∪ has-patch children of v∪D ∪ has-document
children of v∪D`. The union is taken across all inputs.

**Semantics.** Inherently inclusive (today's algorithm already preserves
the inputs). `exclusive` is rejected with the hint *"scope is inherently
inclusive"*. `rel=` and `transitive` are also rejected. Non-issue ids in
the input are skipped by the scope expansion itself; non-issue inputs
still flow through to the union.

**Lowering shape.** **Three** `/v1/relations` calls (the only stage that
costs more than one DB query):

1. `GET /v1/relations?source_ids=<V>&rel_type=child-of&transitive=true` — descendants `D`.
2. `GET /v1/relations?source_ids=<V∪D>&rel_type=has-patch` — patch attachments `P`.
3. `GET /v1/relations?source_ids=<V∪D>&rel_type=has-document` — document attachments `Doc`.

Result: `V ∪ D ∪ P ∪ Doc`.

**Examples.**

```shell
hydra graph search 'i-x | scope'
hydra graph search 'i-x, i-y | scope'
hydra graph search 'i-x | scope | kind=patch'
```

### `kind=K[,K…]`

**Signature.** Client-side post-filter. Retains only nodes in `V` whose
kind is in the list. Applied **after** hydration (kinds aren't known
until the resolver has fetched each node).

**Semantics.** Multiple consecutive `kind=` stages collapse to the
**intersection** of their kind lists at lowering time — `kind=patch |
kind=patch,document` produces a single post-filter for `{patch}`;
`kind=patch | kind=document` produces an empty post-filter. The
`exclusive` flag is not meaningful and not accepted.

**Lowering shape.** Zero HTTP calls (filter is applied client-side).

**Examples.**

```shell
hydra graph search 'i-x | scope | kind=patch'
hydra graph search 'i-x | scope | kind=patch,document'
hydra graph search 'i-x | scope | kind=issue | children rel=has-patch'
```

## Mapping from old flags

Every flag combination accepted by the pre-DSL `Selection` validator has a
one-line DSL counterpart. The DSL's inclusive-by-default contract is
**new**; the old flags excluded the seed from the result, so for an exact
behavioral port (rather than the more natural inclusive form), add
`exclusive`.

| Today | New (closest inclusive) | New (exact today's behavior) |
|---|---|---|
| `--object i-x` | `i-x` (bare-id hydrate) | `i-x \| neighbors exclusive` |
| `--object i-x --rel-type refers-to` | `i-x \| neighbors rel=refers-to` | `i-x \| neighbors rel=refers-to exclusive` |
| `--source i-x` | `i-x \| children` | `i-x \| children exclusive` |
| `--target i-x` | `i-x \| parents` | `i-x \| parents exclusive` |
| `--source i-x --rel-type child-of` | `i-x \| children rel=child-of` | `i-x \| children rel=child-of exclusive` |
| `--target i-x --rel-type child-of` | `i-x \| parents rel=child-of` | `i-x \| parents rel=child-of exclusive` |
| `--source i-x --rel-type child-of --transitive` | `i-x \| children rel=child-of transitive` *or* `i-x \| descendants rel=child-of` | add `exclusive` to either form |
| `--target i-x --rel-type child-of --transitive` | `i-x \| parents rel=child-of transitive` *or* `i-x \| ancestors rel=child-of` | add `exclusive` to either form |
| `--scope i-x` | `i-x \| scope` | n/a — `scope` is inherently inclusive |
| `--kind patch` (combined with any above) | `… \| kind=patch` | unchanged |
| `--kind patch --kind document` | `… \| kind=patch,document` | unchanged |

Combinations the DSL also rejects (carried forward from the old validator
or required by the server):

- `transitive` on `neighbors` — the server can't do transitive closure
  with `object_id`/`object_ids`.
- `transitive` on `parents` / `children` without `rel=` — server
  constraint.
- `exclusive` on `scope` — scope is inherently inclusive.
- A stage with no preceding source — the first element must be a
  `SOURCE`; relation and filter stages cannot lead the query.
- `scope` with `rel=` or `transitive` — `scope` accepts no args.
- Explicit `transitive` on `ancestors` / `descendants` — implicit;
  parser hints at the canonical `parents` / `children` form.
- Duplicate args within a stage (`children exclusive exclusive`,
  `children rel=child-of rel=has-patch`, etc.).

## Error catalog

Every parse error renders the offending token with a caret block, plus a
hint when one applies. Every row below cites the parser test in
`hydra-common/src/graph/query.rs` that pins the error's exact message;
read the test for the canonical rendered form.

### Stage validation

| Error | Trigger | Hint | Parser test |
|---|---|---|---|
| `'transitive' is not supported on 'neighbors'` | `i-x \| neighbors transitive` | use `parents` / `children` with `rel=` for direction-typed transitive walks | `fails_transitive_on_neighbors` |
| `'transitive' on 'parents' requires 'rel='` | `i-x \| parents transitive` | `add a rel filter: 'parents rel=child-of transitive'` | `fails_transitive_without_rel_parents` |
| `'transitive' on 'children' requires 'rel='` | `i-x \| children transitive` | `add a rel filter: 'children rel=child-of transitive'` | `fails_transitive_without_rel_children` |
| `'transitive' is implicit on 'ancestors'` | `i-x \| ancestors rel=child-of transitive` | drop `transitive` (or use the explicit `parents rel=… transitive` form) | `fails_explicit_transitive_on_ancestors` |
| `'transitive' is implicit on 'descendants'` | `i-x \| descendants rel=child-of transitive` | drop `transitive` (or use the explicit `children rel=… transitive` form) | `fails_explicit_transitive_on_descendants` |
| `'ancestors' requires 'rel='` | `i-x \| ancestors` | `e.g., 'ancestors rel=child-of'` | `fails_ancestors_without_rel` |
| `'descendants' requires 'rel='` | `i-x \| descendants` | `e.g., 'descendants rel=child-of'` | `fails_descendants_without_rel` |
| `'exclusive' is not accepted on 'scope'` | `i-x \| scope exclusive` | `scope is inherently inclusive` | `fails_exclusive_on_scope` |
| `'transitive' is not accepted on 'scope'` | `i-x \| scope transitive` | (none) | (covered by scope-rejects-args path) |
| `'rel=' is not accepted on 'scope'` | `i-x \| scope rel=child-of` | (none) | `fails_scope_with_rel` |

### Source errors

| Error | Trigger | Hint | Parser test |
|---|---|---|---|
| `expected source id at start of query` | `""` (empty input) or non-word leading token | (none) | `fails_empty_input` |
| `stage 'X' has no preceding source` | `neighbors` (stage name in source slot) | `queries must start with a source id (e.g., 'i-abcdef \| scope')` | `fails_stage_without_source` |
| `filter stage 'kind=' has no preceding source` | `kind=patch` (filter in source slot) | `queries must start with a source id (e.g., 'i-abcdef \| kind=patch')` | (covered by parser path) |
| `expected source id after ','` | `i-x,` | (none) | (covered by parser path) |
| `invalid source id: <err>` | `not-an-id`, `garbage-text-here` | (none) | `fails_invalid_source_id` |

### Stage shape errors

| Error | Trigger | Hint | Parser test |
|---|---|---|---|
| `expected stage after '\|'` | `i-x \|` (trailing pipe) | (none) | (covered by parser path) |
| `expected stage name after '\|'` | non-word token after `\|` | (none) | (covered by parser path) |
| `unknown stage '<name>'` | `i-x \| kids` | Levenshtein-≤2 or alias hint (`'kids' → 'children'`, etc.) | `fails_unknown_stage_with_levenshtein_kids`, `..._neighbour`, `..._parent` |
| `expected '\|' between stages` | junk where a `\|` should be | (none) | (covered by parser path) |

### Arg errors

| Error | Trigger | Hint | Parser test |
|---|---|---|---|
| `duplicate argument 'transitive'` | `children rel=child-of transitive transitive` | (none) | `fails_duplicate_transitive` |
| `duplicate argument 'exclusive'` | `children exclusive exclusive` | (none) | `fails_duplicate_exclusive` |
| `duplicate argument 'rel='` | `children rel=child-of rel=has-patch` | (none) | `fails_duplicate_rel` |
| `expected '=' after 'rel'` | `children rel transitive` | (none) | (covered by parser path) |
| `expected rel type after 'rel='` | `children rel=` (eof) | (none) | (covered by parser path) |
| `unknown rel type '<word>'` | `neighbors rel=refers_to` | `did you mean 'refers-to'?` (or list of known rel types) | `fails_unknown_rel_type_underscore_form` |
| `unexpected filter stage 'kind=' inside '<name>' stage` | `children kind=patch` (forgotten pipe) | `use '\|' to separate stages: '... \| kind=patch'` | (covered by parser path) |
| `unknown argument '<w>' in '<name>' stage` | `children foo` | optional Levenshtein hint for stage-name typos with missing pipe | (covered by parser path) |
| `unexpected punctuation in stage` | comma or `=` outside `kind=` / `rel=` | (none) | (covered by parser path) |

### Kind filter errors

| Error | Trigger | Hint | Parser test |
|---|---|---|---|
| `expected '=' after 'kind'` | `kind` without `=` | `filter stages are written as 'kind=patch' or 'kind=patch,document'` | `fails_kind_without_eq` |
| `expected kind name after 'kind='` | `kind=` (eof) | (none) | (covered by parser path) |
| `expected kind name` | non-word token after `kind=` | (none) | (covered by parser path) |
| `unknown kind '<word>'` | `kind=widget` | `known kinds: issue, patch, document, conversation` | `fails_unknown_kind` |
| `duplicate kind '<word>' in kind= list` | `kind=patch,patch` | (none) | (covered by parser path) |

### Sample rendered error

```text
$ hydra graph search 'i-abc123 | kids'
error: unknown stage 'kids' at position 11
  i-abc123 | kids
             ^^^^
hint: did you mean 'children'?
```

The carets point at the offending token in the input string. Position is
a byte offset into the (pre-shell-stripped) query.

## Lowering reference

The parser produces a `LoweredQuery { source, stages }` where each
`LoweredStage` is one of:

- `Relations(RelationsQuery { direction, rel, transitive, exclusive })` —
  one `GET /v1/relations` call.
- `Scope` — the three-call scope expansion (runtime-resolved).
- `Kind(Vec<ObjectKind>)` — a post-hydration filter.

The CLI-side resolver walks the lowered stage list against an evolving
vertex set `V`, applying the inclusive-by-default contract per stage.
`ancestors` and `descendants` are collapsed to
`Relations { direction, transitive: true, … }` at lowering time, so the
resolver only ever sees `Parents` / `Children` / `Object` directions.

Per-stage HTTP request shape after lowering, in the order the resolver
issues them:

| Stage (lowered) | Request | `V'` (inclusive) | `V'` (exclusive) | DB queries |
|---|---|---|---|---|
| `parents` over `V` | `GET /v1/relations?target_ids=<V>[&rel_type=R][&transitive=true]` | `V ∪ { r.source_id : r ∈ response }` | `{ r.source_id : r ∈ response } \ V` | 1 |
| `children` over `V` | `GET /v1/relations?source_ids=<V>[&rel_type=R][&transitive=true]` | `V ∪ { r.target_id : r ∈ response }` | `{ r.target_id : r ∈ response } \ V` | 1 |
| `neighbors` over `V` (multi) | `GET /v1/relations?object_ids=<V>[&rel_type=R]` | `V ∪ T(V)` | `T(V) \ V` | 1 |
| `neighbors` over `V` (single) | `GET /v1/relations?object_id=<v>[&rel_type=R]` | `V ∪ T(V)` | `T(V) \ V` | 1 |
| `ancestors rel=R` over `V` | identical to `parents rel=R transitive` | `V ∪ { source_id }` | `{ source_id } \ V` | 1 |
| `descendants rel=R` over `V` | identical to `children rel=R transitive` | `V ∪ { target_id }` | `{ target_id } \ V` | 1 |
| `scope` over `V` | three calls (descendants via child-of, has-patch children, has-document children) | `V ∪ D ∪ P ∪ Doc` | n/a | 3 |
| `kind=…` | (none — client-side filter post-hydration) | `{ v ∈ V : kind(v) ∈ kinds }` | n/a | 0 |

Where `T(V) = { r.source_id : r ∈ response } ∪ { r.target_id : r ∈ response }` for
the relevant per-stage response.

Each relation stage's vertex-set update is a set operation, so the result
is always deduplicated. Iteration / render order is sorted by id at
hydration time and is not influenced by upstream traversal order.

Bare-id fast path: a single-element source with no following stages skips
`/v1/relations` entirely and goes straight to hydration.

## Cost model

The DSL is designed so that every pipe stage corresponds to **at most one
`/v1/relations` call** against the server. The only exception is `scope`,
which preserves the existing three-call algorithm (descendants, then
has-patch children, then has-document children). The `kind=` filter is a
client-side post-filter and issues zero calls. The single-DB-query-per-
stage invariant is what motivates the server-side `object_ids` plural
parameter (added in PR 1 of the migration): without it, multi-element
`neighbors` would require two parallel HTTP calls. Hydration is a
separate `O(|terminal set|)` per-kind concern; the per-stage cost
discussion above does not include it.

Per-stage DB-query counts (lowered):

- `parents`, `children`, `neighbors`, `ancestors`, `descendants`: **1**
- `scope`: **3**
- `kind=`: **0**
- Bare-id source, no stages: **0** (skips `/v1/relations`)

## Shell-quoting tips

The query contains characters the shell treats specially. **Single-quote
the whole query** to pass it through verbatim — the pipe character (`|`)
is the riskiest unquoted token because an unquoted `|` becomes a shell
pipe, silently feeding the rest of the query into nothing.

### bash / zsh / sh

```shell
# Recommended: single quotes around the whole query.
hydra graph search 'i-abc123 | scope | kind=patch'

# When the query contains a shell-expanded variable, use double quotes
# (single quotes would block expansion):
hydra graph log "$HYDRA_ISSUE_ID | scope" --since -7d --verbosity 2
```

### PowerShell

```shell
# Single quotes are literal in PowerShell (no expansion); use double
# quotes when you need `$Var` expansion, the same way as bash.
hydra graph search 'i-abc123 | scope | kind=patch'
hydra graph log "$Env:HYDRA_ISSUE_ID | scope" --since -7d --verbosity 2
```

Common foot-guns:

- An unquoted `|` becomes a shell pipe — the rest of the query is fed to
  the next shell command (often nothing) and the parser sees only the
  source id. Symptom: the query "works" but returns just the bare-id
  hydration.
- An unquoted `,` is usually fine, but the unquoted `=` after `kind=` /
  `rel=` is fine too (the shell only treats `=` as special at the start
  of a word, in assignments).
- An unquoted `(` / `)` would be interpreted by the shell. The DSL has
  no parenthesized forms, so this only comes up if you mistakenly try
  the old atom-form syntax (`neighbors(i-x)`) — the parser rejects it,
  but only after the shell has already complained.
