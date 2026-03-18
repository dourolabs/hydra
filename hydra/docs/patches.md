# `hydra patches`

`hydra patches` captures code changes as server-managed patch artifacts, links them to issues/jobs, and pushes a GitHub branch. GitHub PRs are created automatically server-side when `branch_name` is set on a patch. Patches back Hydra's PR flow, merge queues, and asset uploads.

## Authentication & output

All subcommands honor global CLI flags such as `--server-url`, `--token`, and `--output-format`. Pretty output prints concise tables, while `--output-format jsonl` streams machine-friendly objects (including full diffs). Auth tokens must allow access to the target repository and Hydra tenant.

## How patches relate to jobs and issues

- Each patch stores the git diff, status, and `service_repo_name` pulled from the owning job (`--job` or `HYDRA_ID`). This lets automation trace changes back to the bundle that spawned them.
- Patches are always tied to an issue id (`--issue-id`, defaults to `HYDRA_ISSUE_ID`). Merge-request tracking issues are created automatically by a server-side automation when a patch is created.
- Status updates (e.g., `Open`, `ChangesRequested`) flow through `hydra patches update` and power dashboards, merge queues, and reminder jobs.

## Git workflow expectations

- Work inside a repo that already has a `hydra/<ISSUE_ID>/base` branch pointing at the review baseline. `hydra patches create` compares `hydra/<issue>/base..HEAD` unless you override with `--range`.
- The CLI uses libgit2 for all git operations. It will refuse to create a patch when uncommitted changes exist unless `--allow-uncommitted` is set.
- The CLI always attempts to push the current branch to the remote (using a GitHub token from the server when available). The server-side `GithubPrSyncAutomation` then automatically creates or updates a GitHub PR whenever a patch is created or updated with `branch_name` set.
- Use `hydra patches apply <PATCH_ID>` to pull a teammate's diff into your local tree; it applies the stored patch text onto your current branch without touching remotes.

## Subcommands

### List

```bash
hydra patches list [--id <PATCH_ID> | --query <QUERY>]
```

- `--id` fetches a specific patch record (including reviews and metadata).
- `--query` performs fuzzy search across titles, descriptions, and reviewers.

### Create

```bash
hydra patches create \
  --title "Fix migrations" \
  --description "Explain the schema drift" \
  --issue-id i-123 \
  [--job t-456] \
  [--range base..HEAD] \
  [--allow-uncommitted]
```

- `--title`/`--description` are required and trimmed server-side.
- `--job` (or `HYDRA_ID`) provides the service repo name; omit only for purely local diffs.
- `--issue-id` (default `HYDRA_ISSUE_ID`) determines which task the patch belongs to and drives default commit range selection.
- `--range` overrides the diff base; otherwise Hydra uses `hydra/<issue>/base..HEAD`.
- `--allow-uncommitted` bypasses the clean-tree check when you intentionally want to snapshot staged work-in-progress.

### Apply

```bash
hydra patches apply <PATCH_ID>
```

Downloads the patch diff and applies it to the current repository root. Use this to sync with another agent's work without manually cherry-picking.

### Review

```bash
hydra patches review <PATCH_ID> \
  --author "you@example.com" \
  --contents "Looks great!" \
  [--approve]
```

Adds a timestamped review entry to the patch. Include `--approve` to mark it as approved; omit to leave informational feedback or change requests.

### Update

```bash
hydra patches update <PATCH_ID> \
  [--title "New title"] \
  [--description "More details"] \
  [--status Open|ChangesRequested|Merged]
```

Requires at least one field. Use this to reflect review outcomes or edit metadata before landing the patch.

### Merge queue

```bash
hydra patches merge \
  --repo dourolabs/metis \
  --branch main \
  [--patch-id p-123]
```

- With only `--repo/--branch`, the command prints the current merge queue for that branch.
- Supplying `--patch-id` enqueues the patch for automated merging against the specified repo/branch pair.

### Assets

```bash
hydra patches assets create \
  --patch-id p-123 \
  screenshots/ui.png
```

Uploads arbitrary files (logs, screenshots, binaries) and returns the hosted URL. These assets surface in the patch details and can be referenced from merge-request issues.

## Examples

```bash
# Create a patch, push a branch, and attach to the current job
env HYDRA_ID=t-wwkhrw HYDRA_ISSUE_ID=i-zmgovr \
  hydra patches create \
    --title "Patches docs" \
    --description "Add reference guide"

# Review a teammate's patch with approval
hydra patches review p-123 --author "teammate" --contents "Ship it" --approve

# Apply another agent's diff locally
hydra patches apply p-abc123

# Upload a UI screenshot to an existing patch
hydra patches assets create --patch-id p-abc123 screenshots/ui.png
```
