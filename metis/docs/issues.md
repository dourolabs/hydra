# `metis issues`

The `metis issues` command drives the complete lifecycle of Metis tasks: listing backlogs, creating and updating work items, inspecting dependencies, and managing per-issue todo lists. Use it any time you need to coordinate with agents or automate project tracking.

## Authentication & output

All subcommands inherit the global `metis` flags such as `--server-url`, `--token`, and `--output-format` (defaults to pretty). Switch to `--output-format jsonl` when you need structured machine-readable output or want to pipe results into other tooling. Pretty output shows truncated descriptions and progress notes, while JSONL preserves the full payload.

## Subcommands

### List

```bash
metis issues list \
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

### Describe

```bash
metis issues describe <ISSUE_ID> [--verbose]
```

Summarizes an issue along with its immediate parents, transitive children, todo list, linked patches, and the complete activity log.

#### JSONL output format

In JSONL mode (`--output-format jsonl`), `describe` emits multi-line JSONL with one JSON object per line, ordered by importance so agents can stop reading early:

1. **issue** (1 line) — The issue itself with full details and linked patches.
2. **parent** (0-N lines) — One per parent with compact summary fields.
3. **child** (0-N lines) — One per child with compact summary fields.
4. **activity** (0-N lines) — Activity log entries, one per line.

Each line has a `type` discriminant field:

```jsonl
{"type":"issue", "issue_id":"i-abc", "version":1, "issue":{...}, "patches":[...]}
{"type":"parent", "issue_id":"i-def", "issue_type":"task", "status":"open", "description":"..."}
{"type":"child", "issue_id":"i-ghi", "issue_type":"bug", "status":"in_progress", "assignee":"dev", "description":"...", "progress":"..."}
{"type":"activity", "object_id":"i-abc", "object_kind":"issue", ...}
```

Use `--verbose` to emit multi-line JSONL with full expanded objects per line (full `IssueWithPatches` for parent/child lines, raw `ActivityLogEntry` for activity lines) for automation or auditing.

### Create

```bash
metis issues create \
  [--type <bug|feature|task|chore|merge-request>] \
  [--status <open|in-progress|closed>] \
  [--assignee <USERNAME>] [--progress "text"] \
  [--deps TYPE:ISSUE_ID ...] [--patches PATCH_ID[,PATCH_ID...]] \
  [--repo-name ORG/REPO] [--remote-url URL] [--image IMAGE] \
  [--model MODEL] [--branch BRANCH] [--max-retries N] \
  [--current-issue-id ISSUE_ID] \
  "DESCRIPTION"
```

Descriptions are required; progress defaults to an empty string but may be set inline. Dependencies follow the `TYPE:ISSUE_ID` format (e.g. `child-of:i-abcd`, `blocked-on:i-efgh`); pass `--deps` multiple times to add more than one relationship. `--patches` takes a comma-separated list of existing patch ids. Job settings fields let you pin future jobs to a repo, container image, or branch; inheriting via `--current-issue-id` keeps child tasks aligned with their parent issue’s execution environment.

### Update

```bash
metis issues update <ISSUE_ID> \
  [--type <TYPE>] [--status <STATUS>] \
  [--assignee <USERNAME> | --clear-assignee] \
  [--description "text"] \
  [--deps TYPE:ISSUE_ID ... | --clear-dependencies] \
  [--patches PATCH_ID[,PATCH_ID...] | --clear-patches] \
  [--progress "text" | --clear-progress] \
  [--repo-name ORG/REPO | --remote-url URL | --image IMAGE \
   | --model MODEL | --branch BRANCH | --max-retries N | --clear-job-settings]
```

Use `metis issues update` to change status, hand off work, refresh descriptions, or rewrite the dependency graph. Each field has a corresponding `--clear-*` flag so you can remove values explicitly (e.g., `--clear-progress` when you wrap up a note). Job settings behave like `create`: provide any subset of overrides or call `--clear-job-settings` to drop inherited execution context.

### Todo

```bash
metis issues todo <ISSUE_ID> [--add "text" | --done N | --undone N | --replace ITEM[,ITEM...]]
```

Append todos with `--add`; prefix the text with `[x]` to mark the entry complete immediately. Use `--done` / `--undone` with 1-based indexes to toggle status, or `--replace` to rewrite the entire ordered list (commas separate items). Pretty output mirrors the dashboard checklist, while `--output-format jsonl` returns `{ issue_id, todo_list }` for scripts.

## Examples

```bash
# File a bug that inherits the current job's repo/image
metis issues create \
  --current-issue-id i-root \
  --type bug --assignee swe --repo-name dourolabs/metis \
  --deps child-of:i-root --progress "Triaging logs" \
  "API times out when payload > 5MB"

# Check everything blocked by a flaky test epic and emit JSON
metis --output-format jsonl issues list --graph "**:blocked-on:i-flaky"

# Move work in progress forward and capture notes
metis issues update i-1234 --status closed --progress "Tests green, patch merged"

# Add a follow-up dependency and a todo item
metis issues update i-1234 \
  --deps child-of:i-parent \
  --deps blocked-on:i-migration
metis issues todo i-1234 --add "[x] Document migration steps"

# Replace todos after a grooming session
metis issues todo i-1234 --replace "Cut RC branch","Invite QA","Prep launch blog"

# Inspect an issue’s relationships and activity log verbosely
metis issues describe i-1234 --verbose
```
