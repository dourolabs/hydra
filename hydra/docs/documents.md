# `hydra documents`

The `hydra documents` command mirrors the `hydra patches` UX, but targets markdown documents stored on the Hydra server. These documents are typically used for runbooks, RFCs, or job-generated notes.

## Authentication & output

All subcommands honor global `hydra` flags such as `--server-url`, `--token`, and `--output-format` (pretty/jsonl). Pretty output truncates long bodies to 20 lines; switch to `--output-format jsonl` to inspect entire payloads.

## Subcommands

### List

```bash
hydra documents list [--query <QUERY>] [--path-prefix <PREFIX>]
```

* `--query` does a fuzzy match on titles and body text.
* `--path-prefix` filters hierarchical paths such as `docs/runbooks/`.

### Get

```bash
hydra documents get <DOCUMENT_ID>
```

Fetches a single record (including the latest body) by id.

### Create

```bash
hydra documents create \
  --title "Runbook" \
  --path docs/runbooks/db.md \
  --body-file ./db.md
```

Body sources (pick exactly one):

1. `--body "markdown"`
2. `--body-file path/to/file.md`
3. `--body-stdin` (or pipe data via stdin, e.g. `cat doc.md | hydra documents create ...`).

Paths must be non-empty strings when provided.

### Update

```bash
hydra documents update <DOCUMENT_ID> \
  [--title "New Title"] \
  [--body "markdown" | --body-file FILE | --body-stdin] \
  [--path NEW_PATH | --clear-path]
```

At least one field must change. `--clear-path` removes the stored path.

## Examples

```bash
# Pipe edited markdown from stdin
sed 's/TODO/Done/' runbook.md | hydra documents update d-xyz --body-stdin

# List docs under a path prefix with pretty output
hydra documents list --path-prefix docs/

# Fetch JSONL for automation
hydra --output-format jsonl documents get d-xyz
```
