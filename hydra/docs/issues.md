# `hydra issues`

The `hydra issues` command drives the complete lifecycle of Hydra tasks: listing backlogs, creating and updating work items, and inspecting dependencies. Use it any time you need to coordinate with agents or automate project tracking.

## Authentication & output

All subcommands inherit the global `hydra` flags such as `--server-url`, `--token`, and `--output-format` (defaults to pretty). Switch to `--output-format jsonl` when you need structured machine-readable output or want to pipe results into other tooling. Pretty output shows truncated descriptions, while JSONL preserves the full payload.

## Subcommands

### List

```bash
hydra issues list \
  [--id ISSUE_ID] [--type <TYPE>] [--status <STATUS>] [--assignee <USERNAME>] \
  [--query <TEXT>] [--graph <FILTER>[,<FILTER>...]]
```

* `--id` fetches a single issue (and ignores other filters).
* `--type`, `--status`, and `--assignee` narrow the search by metadata; assignees are case-insensitive.
* `--query` performs a fuzzy search across titles and descriptions.
* `--graph` accepts one or more dependency filters using the syntax `<SELECTOR>:<DEPENDENCY>:<SELECTOR>` where exactly one selector is a wildcard:
  * `*` = immediate neighbors (one hop) and `**` = transitive traversal.
  * The non-wildcard selector is an issue id (`i-1234`).
  * `<DEPENDENCY>` is any supported relationship such as `child-of` or `blocked-on`.
  * Examples: `*:child-of:i-root` lists issues whose parent is `i-root`, while `i-leaf:blocked-on:**` finds everything the leaf depends on (recursively).

### Create

```bash
hydra issues create \
  [--type <bug|feature|task|chore|merge-request>] \
  [--status <open|in-progress|closed>] \
  [--assignee <USERNAME>] \
  [--deps TYPE:ISSUE_ID ...] [--patches PATCH_ID[,PATCH_ID...]] \
  [--repo-name ORG/REPO] [--remote-url URL] [--image IMAGE] \
  [--model MODEL] [--branch BRANCH] [--max-retries N] \
  [--current-issue-id ISSUE_ID] \
  "DESCRIPTION"
```

Descriptions are required. Dependencies follow the `TYPE:ISSUE_ID` format (e.g. `child-of:i-abcd`, `blocked-on:i-efgh`); pass `--deps` multiple times to add more than one relationship. `--patches` takes a comma-separated list of existing patch ids. Job settings fields let you pin future jobs to a repo, container image, or branch; inheriting via `--current-issue-id` keeps child tasks aligned with their parent issue’s execution environment.

### Update

```bash
hydra issues update <ISSUE_ID> \
  [--type <TYPE>] [--status <STATUS>] \
  [--assignee <USERNAME> | --clear-assignee] \
  [--description "text"] \
  [--deps TYPE:ISSUE_ID ... | --clear-dependencies] \
  [--patches PATCH_ID[,PATCH_ID...] | --clear-patches] \
  [--repo-name ORG/REPO | --remote-url URL | --image IMAGE \
   | --model MODEL | --branch BRANCH | --max-retries N | --clear-job-settings]
```

Use `hydra issues update` to change status, hand off work, refresh descriptions, or rewrite the dependency graph. Each field has a corresponding `--clear-*` flag so you can remove values explicitly. Job settings behave like `create`: provide any subset of overrides or call `--clear-job-settings` to drop inherited execution context.

## Examples

```bash
# File a bug that inherits the current job's repo/image
hydra issues create \
  --current-issue-id i-root \
  --type bug --assignee swe --repo-name dourolabs/hydra \
  --deps child-of:i-root \
  "API times out when payload > 5MB"

# Check everything blocked by a flaky test epic and emit JSON
hydra --output-format jsonl issues list --graph "**:blocked-on:i-flaky"

# Move work in progress forward
hydra issues update i-1234 --status closed

# Add follow-up dependencies
hydra issues update i-1234 \
  --deps child-of:i-parent \
  --deps blocked-on:i-migration
```
