# `hydra documents`

The `hydra documents` command mirrors the `hydra patches` UX, but targets markdown documents stored on the Hydra server. These documents are typically used for runbooks, RFCs, or job-generated notes.

## Authentication & output

All subcommands honor global `hydra` flags such as `--server-url`, `--token`, and `--output-format` (pretty/jsonl). Pretty output truncates long bodies to 20 lines; switch to `--output-format jsonl` to inspect entire payloads.

## Subcommands

### List

```bash
hydra documents list [--query <QUERY>] [--path-prefix <PREFIX>] [--created-by <TASK_ID>]
```

* `--query` does a fuzzy match on titles and body text.
* `--path-prefix` filters hierarchical paths such as `docs/runbooks/`.
* `--created-by` accepts any Hydra task id (defaults to `HYDRA_ID` when set).

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
  --body-file ./db.md \
  [--created-by <TASK_ID>]
```

Body sources (pick exactly one):

1. `--body "markdown"`
2. `--body-file path/to/file.md`
3. `--body-stdin` (or pipe data via stdin, e.g. `cat doc.md | hydra documents create ...`).

`--created-by` defaults to `$HYDRA_ID` when present so jobs can attribute authored documents. Paths must be non-empty strings when provided.

### Update

```bash
hydra documents update <DOCUMENT_ID> \
  [--title "New Title"] \
  [--body "markdown" | --body-file FILE | --body-stdin] \
  [--path NEW_PATH | --clear-path]
```

At least one field must change. `--clear-path` removes the stored path. Updates preserve `created_by` from the existing record.

## Examples

```bash
# Pipe edited markdown from stdin
sed 's/TODO/Done/' runbook.md | hydra documents update d-xyz --body-stdin

# List docs created by the current job with pretty output
env HYDRA_ID=t-abcd hydra documents list --created-by $HYDRA_ID --path-prefix docs/

# Fetch JSONL for automation
hydra --output-format jsonl documents get d-xyz
```
