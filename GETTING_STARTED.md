# Getting Started with Metis

This guide covers how to download and build Metis from GitHub, how to view running work with the dashboard, and how to create/manage issues with the CLI.

## 1) Download and build from GitHub

```bash
# Clone the repository

git clone https://github.com/dourolabs/metis.git
cd metis

# Build the workspace (CLI + server + shared crates)

cargo build --workspace --all-targets
```

If you just want a quick sanity check that everything compiles, you can also run:

```bash
cargo check --workspace
```

## 2) Start the server and point the CLI at it

The CLI needs a running `metis-server` to show live data and to create issues. A minimal local setup:

```bash
cp metis-server/config.toml.sample metis-server/config.toml
METIS_CONFIG=metis-server/config.toml cargo run -p metis-server
```

In another terminal, point the CLI at the server (default is `http://localhost:8080` if unchanged):

```bash
export METIS_SERVER_URL=http://localhost:8080
```

## 3) Use the dashboard to see issues and jobs in progress

Launch the dashboard UI:

```bash
metis dashboard
```

The dashboard is a live view of jobs, issues, and patches. It refreshes automatically, so you can keep it open while work is running. You can also run `metis` with no subcommand to open the dashboard by default.

## 4) Create and manipulate issues with the CLI

List issues (filter by status, type, etc.):

```bash
metis issues list
metis issues list --status in-progress
```

Create a new issue:

```bash
metis issues create "Investigate slow job startup"
metis issues create --type bug --assignee alice "Jobs get stuck in pending"
```

Update an issue (status, assignee, progress notes, etc.):

```bash
metis issues update i-abc123 --status in-progress --progress "Reproduced on staging"
metis issues update i-abc123 --assignee bob
```

Manage the issue todo list:

```bash
metis issues todo i-abc123 --add "Collect logs"
metis issues todo i-abc123 --done 1
metis issues todo i-abc123 --replace "Collect logs,Add regression test,Write summary"
```

Describe an issue (includes related issues and patches):

```bash
metis issues describe i-abc123
```
