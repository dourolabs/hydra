# dourolabs/metis repository index

## High-level structure
- Rust workspace with crates: `metis` (CLI), `metis-server` (Axum API + background workers), `metis-common` (shared models).
- Supporting directories: `images/` (Dockerfiles), `scripts/` (automation), `metis-ui` and `metis-component-library` (frontend packages).
- Config templates: `config.toml.sample` per crate; copy to `config.toml` for overrides.

## AGENTS.md highlights
- Keep CLI subcommands in `metis/src/command`; `metis-server/AGENTS.md` documents server routes/background layout.
- Use `MetisId` instead of raw `String` for identifiers; CLI git ops must use libgit2.
- If a CLI command needs env vars, declare them on the arg struct via `#[arg(env = ...)]`.
- Required pre-submit checks: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- README should stay high-level; avoid adding CLI command details unless requested.

## README.md highlights
- Metis is an AI-orchestrator: CLI controls an Axum-based control plane that schedules agents on Kubernetes.
- Describes repo layout, prerequisites (Rust, Kubernetes access, Docker, OpenAI key), and build/test commands.
- Configuration guidance for CLI and server, plus local dev workflows (Postgres helper, GitHub App, kind cluster).

## DESIGN.md highlights
- Describes Metis as an agent coordination framework centered on an issue tracker and task graph.
- Issues have explicit statuses (Open, InProgress, Dropped, Closed) plus inferred Ready/NotReady states.
- Agents and humans use the same CLI; agents update issue status and can hand off work across sessions.
- Git state is tracked via `metis/<issue-id|task-id>/{base,head}` branches managed by `worker_run`.
