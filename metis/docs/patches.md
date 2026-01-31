# `metis patches`

`metis patches` captures code changes as server-managed patch artifacts, links them to issues/jobs, and optionally pushes a GitHub branch. Patches back Metis' PR flow, merge queues, and asset uploads.

## Authentication & output

All subcommands honor global CLI flags such as `--server-url`, `--token`, and `--output-format`. Pretty output prints concise tables, while `--output-format jsonl` streams machine-friendly objects (including full diffs). Auth tokens must allow access to the target repository and Metis tenant.

## How patches relate to jobs and issues

- Each patch stores the git diff, status, and `service_repo_name` pulled from the owning job (`--job` or `METIS_ID`). This lets automation trace changes back to the bundle that spawned them.
- Patches are always tied to an issue id (`--issue-id`, defaults to `METIS_ISSUE_ID`). When `--assignee` is provided, the CLI automatically files a merge-request issue that depends on the source task and references the new patch id.
- Status updates (e.g., `Open`, `ChangesRequested`) flow through `metis patches update` and power dashboards, merge queues, and reminder jobs.

## Git workflow expectations

- Work inside a repo that already has a `metis/<ISSUE_ID>/base` branch pointing at the review baseline. `metis patches create` compares `metis/<issue>/base..HEAD` unless you override with `--range`.
- The CLI uses libgit2 for all git operations. It will refuse to create a patch when uncommitted changes exist unless `--allow-uncommitted` is set.
- When `--github` is enabled, Metis fetches a GitHub token from the server, creates (if needed) a feature branch derived from the current branch name or `metis-<job>` slug, and pushes it before recording the patch.
- Use `metis patches apply <PATCH_ID>` to pull a teammate's diff into your local tree; it applies the stored patch text onto your current branch without touching remotes.

## Subcommands

### List

```bash
metis patches list [--id <PATCH_ID> | --query <QUERY>]
```

- `--id` fetches a specific patch record (including reviews and metadata).
- `--query` performs fuzzy search across titles, descriptions, and reviewers.

### Create

```bash
metis patches create \
  --title "Fix migrations" \
  --description "Explain the schema drift" \
  --issue-id i-123 \
  [--job t-456] \
  [--github] \
  [--assignee reviewer] \
  [--range base..HEAD] \
  [--allow-uncommitted]
```

- `--title`/`--description` are required and trimmed server-side.
- `--job` (or `METIS_ID`) provides the service repo name; omit only for purely local diffs.
- `--github` pushes the feature branch and keeps it synced to the patch.
- `--assignee` files a merge-request issue assigned to that username and linked to the parent issue.
- `--issue-id` (default `METIS_ISSUE_ID`) determines which task the patch belongs to and drives default commit range selection.
- `--range` overrides the diff base; otherwise Metis uses `metis/<issue>/base..HEAD`.
- `--allow-uncommitted` bypasses the clean-tree check when you intentionally want to snapshot staged work-in-progress.

### Apply

```bash
metis patches apply <PATCH_ID>
```

Downloads the patch diff and applies it to the current repository root. Use this to sync with another agent's work without manually cherry-picking.

### Review

```bash
metis patches review <PATCH_ID> \
  --author "you@example.com" \
  --contents "Looks great!" \
  [--approve]
```

Adds a timestamped review entry to the patch. Include `--approve` to mark it as approved; omit to leave informational feedback or change requests.

### Update

```bash
metis patches update <PATCH_ID> \
  [--title "New title"] \
  [--description "More details"] \
  [--status Open|ChangesRequested|Merged]
```

Requires at least one field. Use this to reflect review outcomes or edit metadata before landing the patch.

### Merge queue

```bash
metis patches merge \
  --repo dourolabs/metis \
  --branch main \
  [--patch-id p-123]
```

- With only `--repo/--branch`, the command prints the current merge queue for that branch.
- Supplying `--patch-id` enqueues the patch for automated merging against the specified repo/branch pair.

### Assets

```bash
metis patches assets create \
  --patch-id p-123 \
  screenshots/ui.png
```

Uploads arbitrary files (logs, screenshots, binaries) and returns the hosted URL. These assets surface in the patch details and can be referenced from merge-request issues.

## Examples

```bash
# Create and assign a review issue, push a branch, and attach to the current job
env METIS_ID=t-wwkhrw METIS_ISSUE_ID=i-zmgovr \
  metis patches create \
    --title "Patches docs" \
    --description "Add reference guide" \
    --github \
    --assignee jayantk

# Review a teammate's patch with approval
metis patches review p-123 --author "teammate" --contents "Ship it" --approve

# Apply another agent's diff locally
metis patches apply p-abc123

# Upload a UI screenshot to an existing patch
metis patches assets create --patch-id p-abc123 screenshots/ui.png
```
