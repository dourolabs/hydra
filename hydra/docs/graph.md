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

`search` consumes a positional pipe-grammar **query** (described below).
`diff` and `log` still ride on the older selection-flag surface (the cutover
is tracked under issues `i-` PRs 4 and 5); they will switch to the same
query grammar in a follow-up.

## Query grammar (`search`)

`hydra graph search` accepts one positional argument: the **query**. The
shell-friendly convention is to single-quote the whole thing so `|`, `,`,
and `=` are passed verbatim:

```
hydra graph search '<QUERY>'
```

The grammar is a SOURCE followed by zero or more pipe-separated STAGEs that
each transform the running vertex set:

```
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
`--source`/`--target`/`--object` semantics).

`scope` runs the existing 3-call expansion (descendants via `child-of`,
then `has-patch` children of `V ∪ D`, then `has-document` children of
`V ∪ D`). `scope` is inherently inclusive; `exclusive` is rejected.

`kind=` filters apply after hydration; consecutive `kind=` stages collapse
to the intersection of their kind lists.

Parse errors quote the input with a caret and a Levenshtein-≤2 hint when
possible (e.g. `kids` → `children`).

### Worked examples

```bash
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

## Selection flags (`diff` / `log`)

`diff` and `log` still accept the legacy flag-mirror surface until their
cutovers land. Pick the node set with **exactly one** selection mode:

- `--source <ID>` — edges originating at `<ID>`. Combine with `--rel-type`
  and `--transitive` to walk down a relation.
- `--target <ID>` — edges pointing at `<ID>` (e.g. `--target i-root
  --rel-type child-of --transitive` finds all descendants of `i-root`).
- `--object <ID>` — edges where `<ID>` is the source **or** target.
- `--scope <ID>` — convenience: `<ID>` plus all transitive child issues,
  plus their attached patches and documents. Mutually exclusive with
  `--source`/`--target`/`--object`. `refers-to` is intentionally **not**
  fanned out.

Filters that compose with the modes above:

- `--rel-type <TYPE>` — restrict to one relation type (`child-of`,
  `blocked-on`, `has-patch`, `has-document`, `refers-to`, ...).
- `--transitive` — follow the edge type transitively. Requires `--source`
  or `--target` plus `--rel-type`.
- `--kind <KIND>` — post-filter the hydrated nodes to one or more kinds
  (`issue`, `patch`, `document`, `conversation`). Repeatable.

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

```bash
# What changed in the i-root subtree over the last day, full record.
hydra graph diff 'i-root | scope' --since -1d --verbosity 3

# Last 50 created/updated events on i-root and its bundle.
hydra graph log --since -7d --limit 50 --scope i-root
```

## Output format

`hydra graph` honors the global `--output-format` flag. Pretty output
renders a table; `jsonl` emits one record per line (with `kind` and `id`
fields plus the verbosity-projected view merged in).
