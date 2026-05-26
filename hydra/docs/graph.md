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

All three accept the same selection flags described below; `diff` and `log`
add `--since` / `--until`, and `log` adds `--limit`.

## Selection flags

Pick the node set with **exactly one** selection mode:

- `--source <ID>` — edges originating at `<ID>`. Combine with `--rel-type` and
  `--transitive` to walk down a relation.
- `--target <ID>` — edges pointing at `<ID>` (e.g. `--target i-root
  --rel-type child-of --transitive` finds all descendants of `i-root`).
- `--object <ID>` — edges where `<ID>` is the source **or** target.
- `--scope <ID>` — convenience: `<ID>` plus all transitive child issues, plus
  their attached patches and documents. Mutually exclusive with
  `--source`/`--target`/`--object`. `refers-to` is intentionally **not**
  fanned out.

Filters that compose with the modes above:

- `--rel-type <TYPE>` — restrict to one relation type (`child-of`,
  `blocked-on`, `has-patch`, `has-document`, `refers-to`, ...).
- `--transitive` — follow the edge type transitively. Requires `--source` or
  `--target` plus `--rel-type`.
- `--kind <KIND>` — post-filter the hydrated nodes to one or more kinds
  (`issue`, `patch`, `document`, `conversation`). Repeatable.

Safety:

- `--max-nodes <N>` — fail with exit code 2 if the resolved id set exceeds
  `<N>` (default 10 000). Re-run with `--max-nodes` raised, or narrow the
  selection.

## Verbosity

`--verbosity <1|2|3>` controls how much of each node is rendered:

- `1` (default) — terse: id, kind, title, status.
- `2` — intermediate: adds description / progress / assignee for issues, the
  diff for patches, the body for documents.
- `3` — full: the entire record as stored in the server.

## Time-window flags (`diff` / `log` only)

`--since <TS>` is optional on both subcommands; when omitted, it defaults to
the Unix epoch (`1970-01-01T00:00:00Z`), i.e. "from the beginning of time".
`--until <TS>` defaults to `now`. Each accepts:

- RFC 3339 timestamps (`2026-05-19T12:00:00Z`).
- Relative durations (`-2h`, `-7d`, `-30m`).
- The literal `now`.

`hydra graph log` also accepts `--limit <N>` (default 50) to cap the number
of events emitted, most recent first.

## Examples

```bash
# All immediate child issues of i-root (just the edges' other endpoints).
hydra graph search --target i-root --rel-type child-of

# Everything reachable below i-root, transitively, restricted to issues.
hydra graph search \
  --target i-root --rel-type child-of --transitive \
  --kind issue

# The full bundle for a parent issue: itself, descendants, patches, documents.
hydra graph search --scope i-root

# What changed in the i-root subtree over the last day, full record.
hydra graph diff 'i-root | scope' --since -1d --verbosity 3

# Last 50 created/updated events on i-root and its bundle.
hydra graph log --since -7d --limit 50 --scope i-root

# Patches attached to a specific issue.
hydra graph search --source i-root --rel-type has-patch --kind patch
```

## Output format

`hydra graph` honors the global `--output-format` flag. Pretty output renders
a table; `jsonl` emits one record per line (with `kind` and `id` fields plus
the verbosity-projected view merged in).
