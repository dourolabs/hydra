# Getting Started with Metis

This guide covers how to download and build Metis from GitHub, how to view running work with the dashboard, and how to create/manage issues with the CLI.

## 1) Download and build from GitHub

```bash
# Clone the repository

git clone https://github.com/dourolabs/metis.git
cd metis

# Build the CLI

cargo build -p metis
```

Then, add metis to your path:

```bash
export PATH="$PATH:`realpath ./target/debug/`"
```

## 2) Connect to the server

Simply run the metis command and point it to the development server:

```bash
metis --server-url http://metis-development.monster-vibes.ts.net
```

You will be prompted to log in with your github account.
You won't need to provide `--server-url` with future commands (unless you want to connect to a different server).

## 3) Use the dashboard to see issues and jobs in progress

Both `metis` and `metis dashboard` will open the metis dashboard. 
The dashboard is a live view of jobs, issues, and patches.
It refreshes automatically, so you can keep it open while work is running to track progress.

## 4) Create your first issue with CLI

The basic units of work in metis are issues. You create issues, and then agents work on them and submit PRs back to you asynchronously.

To create an issue, you first need to add your repository to metis if it's not already there:

```bash
# See what repos are registered
metis repos list 
# Add your repo if it doesn't exist
metis repos create dourolabs/metis https://github.com/dourolabs/metis.git
```

Once you've added your repo, create an issue by running the following command:

```bash
metis issues create --assignee swe --repo-name your-org/your-repo "please fix the bug in ..."
```

This command assigns the issue to `swe`, which is a software engineering agent.
After running this command, you should see the issue in the dashboard, and the agent picking it up and working on it.
Once the agent is done, it will create an issue assigned to you to review the PR.
The PR is copied to Github -- you can submit your review feedback there.
For a deeper tour of `metis issues`, including when to reach for `list` (backlog triage), `describe` (dependency graph + activity log), `update` (status/progress changes), and `todo` (per-issue checklists), see [metis/docs/issues.md](metis/docs/issues.md).

## 5) Try a more complicated issue

Metis also has a product manager agent named `pm` who can break down more complicated features and projects
into smaller tasks for development. Try writing a more complex feature description and assigning it to the PM:

```bash
metis issues create --assignee pm --repo-name your-org/your-repo "Feature: please build XYZ"
```

As the work unfolds, use `metis issues list` to watch related tasks, `metis issues describe` to understand blockers and linked patches, `metis issues update` to record status changes, and `metis issues todo` to keep hand-off steps visible. Each of these flows is documented with copy/paste snippets in [metis/docs/issues.md](metis/docs/issues.md).

## Maintain Shared Documents

Use `metis documents` to capture design docs, runbooks, and other markdown artifacts in the server store. See [metis/docs/documents.md](metis/docs/documents.md) for a complete tour of the subcommands, input flags, and examples.

## Submit and review patches

Run `metis patches` whenever you are ready to package local commits for review or to fetch a teammate's diff. The command snapshots the diff from your service repository, attaches it to the active issue (`METIS_ISSUE_ID`), and optionally files/assigns a merge-request issue plus a GitHub branch. Before invoking it, make sure:

- Your repository is registered with Metis (`metis repos create ...`) and your CLI is authenticated with both Metis and (if using `--github`) GitHub.
- You have set `METIS_ISSUE_ID` (and typically `METIS_ID`) so the CLI can locate the job context and base branch.
- Your working tree has the commits you intend to send; use `--allow-uncommitted` only for deliberate WIP snapshots.

See [metis/docs/patches.md](metis/docs/patches.md) for the full reference covering subcommands, merge queues, and asset uploads.
