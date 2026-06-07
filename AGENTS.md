# Repository Guidelines

## Reference docs

Per-topic reference clusters under `docs/`. Browse what's relevant — none of
this is required reading.

- [docs/testing.md](docs/testing.md) — Cross-cutting testing rules: TDD is required, and never widen exports for tests.
- [docs/rust/AGENTS.md](docs/rust/AGENTS.md) — Rust workspace standards: style, idioms, errors/logging, testing, CLI conventions.
- [docs/typescript/AGENTS.md](docs/typescript/AGENTS.md) — Frontend (`hydra-web/`) standards: workspace shape, CSS Modules, React Query + SSE, integration testing.
- [docs/architecture/AGENTS.md](docs/architecture/AGENTS.md) — Architectural standards: issue/graph model, sessions + git branches, routes/domain/store layering, automations vs. background workers, API wire contract.
- [docs/open-core.md](docs/open-core.md) — Dual-license layout (MIT core + proprietary `ee/`), cargo features (`postgres`, `kubernetes`, `enterprise`, `test-utils`), and Postgres migration baselines.

## Project Structure & Module Organization
Workspace crates: `hydra` (CLI), `hydra-server` (Axum API and background workers), and `hydra-common` (shared models). CLI subcommands stay under `hydra/src/command`; see `hydra-server/AGENTS.md` for detailed route and background layout expectations. Copy each crate's sample config (`config.toml.sample` for hydra CLI; `config.yaml.sample` for hydra-server) to `config.toml` or `config.yaml` respectively for overrides. Dockerfiles live in `images/`; automation scripts are in `scripts/`.

## Frontend Development
For frontend development and visual testing, see `hydra-web/AGENTS.md`.

## Build, Test, and Development Commands
- `cargo check --workspace` quickly verifies the entire workspace compiles.
- `cargo build --workspace --all-targets` produces debug binaries; add `--release` when publishing images.
- `cargo run -p hydra -- sessions list` runs the CLI against a server; substitute other subcommands from `hydra/src/command`.
- `HYDRA_CONFIG=hydra-server/config.yaml cargo run -p hydra-server` launches the HTTP service with the desired config.
- `./scripts/docker-build.sh` builds all deployment containers.

## Documentation Guidelines
- Do not add CLI command details to `README.md` unless explicitly requested; the README has tight space and should stay focused on top-level orientation, so keep command-specific docs elsewhere.

## Final Task Checklist
Before finishing any task, you **must** run and fix all issues from these commands:
- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

## Commit & Pull Request Guidelines
Please use proper capitalization and sentences. Keep pull request descriptions short and to the point: provide motivation / context for the change, explain what changed, and explain how it was tested.
Please explicitly call out anything that may be confusing or design questions where you made an explicit
choice with tradeoffs, and what the alternatives were. Attach screenshots or CLI snippets for UX changes and highlight configuration, migration, or security impacts.
- **Do not commit screenshots or other images to the git repository**, except for images used in repository documentation (e.g., README screenshots). Documentation images should be placed in `docs/images/`. For all other screenshots, upload them to the hydra document store under the `screenshots/` directory.

## Configuration & Security Notes
Never commit secrets. Use the sample config files (`config.toml.sample` or `config.yaml.sample`) as templates and load them via `HYDRA_CONFIG` or env vars such as `OPENAI_API_KEY`. Confirm Docker images reference the intended worker image and namespace before publishing. Add new external integrations to `hydra-common` so sensitive values stay centralized and masked.
