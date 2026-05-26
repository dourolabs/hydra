# `hydra graph`

The `hydra graph` command queries Hydra's knowledge graph. The graph's **nodes**
are issues, patches, documents, and conversations; the **edges** are typed
relations between them (e.g. `child-of`, `blocked-on`, `has-patch`,
`has-document`, `refers-to`). The three subcommands return hydrated nodes —
not raw edges — projected through version-aware views.

This command replaces an older, edge-oriented CLI surface. The output shape
is different: the previous command returned a list of raw edges; `hydra
graph` returns the set of nodes the matching edges touch (or, for `diff` /
`log`, deltas over time on those nodes). Migration tip: if you were piping
edges, switch to `hydra graph search` and consume the node records.

## Subcommands

- `search` — return the current state of the matched node set.
- `diff` — return added / removed / modified records over a time window.
- `log` — stream a time-ordered event log of `created` / `updated` records.

All three subcommands take a single positional **query** argument (the
pipe-grammar DSL described below). `diff` and `log` additionally accept
`--since` / `--until` for the time window; `log` accepts `--limit` to cap
emitted events.

## Query grammar

The selection grammar is shared verbatim across `search`, `diff`, and
`log`. The shell-friendly convention is to single-quote the whole thing so
`|`, `,`, and `=` pass through to the parser unmolested:

```shell
hydra graph search '<QUERY>'
hydra graph diff   '<QUERY>' --since -7d
hydra graph log    '<QUERY>' --since -7d --limit 50
```

The grammar is a SOURCE followed by zero or more pipe-separated STAGEs
that each transform the running vertex set:

```text
QUERY     := SOURCE ('|' STAGE)*
SOURCE    := ID (',' ID)*
STAGE     := RELATION_STAGE | FILTER_STAGE
RELATION_STAGE := NAME [ARG ...]
NAME      := parents | children | neighbors | ancestors | descendants | scope
ARG       := rel=RELTYPE | transitive | exclusive
FILTER_STAGE   := kind=KIND[,KIND...]
RELTYPE   := child-of | blocked-on | has-patch | has-document | refers-to
KIND      := issue | patch | document | conversation
```

All five relation stages default to **inclusive**: `V | stage = V ∪
traversal(V)` — the seed is preserved even when the traversal returns no
rows. Add `exclusive` to drop the seed (matches the older flag surface's
`--source` / `--target` / `--object` semantics).

`scope` runs the existing 3-call expansion (descendants via `child-of`,
then `has-patch` children of `V ∪ D`, then `has-document` children of
`V ∪ D`). `scope` is inherently inclusive; `exclusive` is rejected.

`kind=` filters apply after hydration; consecutive `kind=` stages collapse
to the intersection of their kind lists.

Parse errors quote the input with a caret and a Levenshtein-≤2 hint when
possible (e.g. `kids` → `children`).

The full reference — every stage's arg rules, every parse error with its
hint, the full mapping table from the old flag surface, the per-stage
HTTP-request shape, and shell-quoting examples — lives in
[`graph-query.md`](graph-query.md).

### Worked examples

```shell
# A single issue (bare-id fast path — no /v1/relations call).
hydra graph search 'i-root'

# i-root plus all its direct children (default inclusive).
hydra graph search 'i-root | children rel=child-of'

# Everything reachable below i-root via child-of, transitively, restricted
# to issues.
hydra graph search 'i-root | descendants rel=child-of | kind=issue'

# The full bundle for a parent issue: itself, descendants, patches, documents.
hydra graph search 'i-root | scope'

# Patches attached to a parent issue.
hydra graph search 'i-root | scope | kind=patch'

# i-root plus its refers-to neighbors, then their parents via child-of.
hydra graph search 'i-root | neighbors rel=refers-to | parents rel=child-of'

# Union of two scopes.
hydra graph search 'i-root1, i-root2 | scope'
```

## Common flags

Safety / shape, accepted by all three subcommands:

- `--max-nodes <N>` — fail with exit code 2 if the resolved id set exceeds
  `<N>` (default 10 000). Re-run with `--max-nodes` raised, or narrow the
  selection.
- `--verbosity <1|2|3>` — controls how much of each node is rendered:
  - `1` (default) — terse: id, kind, title, status.
  - `2` — intermediate: adds description / progress / assignee for issues,
    the diff for patches, the body for documents.
  - `3` — full: the entire record as stored in the server.

## Time-window flags (`diff` / `log` only)

`--since <TS>` is optional on both subcommands; when omitted, it defaults
to the Unix epoch (`1970-01-01T00:00:00Z`), i.e. "from the beginning of
time". `--until <TS>` defaults to `now`. Each accepts:

- RFC 3339 timestamps (`2026-05-19T12:00:00Z`).
- Relative durations (`-2h`, `-7d`, `-30m`).
- The literal `now`.

`hydra graph log` also accepts `--limit <N>` (default 50) to cap the number
of events emitted, most recent first.

## Examples (`diff` / `log`)

```shell
# What changed in the i-root subtree over the last day, full record.
hydra graph diff 'i-root | scope' --since -1d --verbosity 3

# Last 50 created/updated events on i-root and its bundle.
hydra graph log 'i-root | scope' --since -7d --limit 50
```

## Output format

`hydra graph` honors the global `--output-format` flag. Pretty output
renders a table; `jsonl` emits one record per line (with `kind` and `id`
fields plus the verbosity-projected view merged in).

## See also

- [`graph-query.md`](graph-query.md) — long-form reference for the query
  DSL: every stage's args, every parse error, the mapping from the old
  flag surface, and the per-stage HTTP request shape.
- `/designs/hydra-graph-query-language.md` (document store) — design
  rationale for the DSL and the migration plan.
- `/designs/hydra-graph-cli.md` (document store) — historical design for
  the broader `hydra graph` surface; the node-selection sections are
  superseded by the query DSL design above, but the time-window /
  verbosity / output-format sections remain in force.
