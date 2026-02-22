# Repository Guidelines

## Project Structure & Module Organization
Workspace crates: `metis` (CLI), `metis-server` (Axum API and background workers), and `metis-common` (shared models). CLI subcommands stay under `metis/src/command`; see `metis-server/AGENTS.md` for detailed route and background layout expectations. Copy each crate's sample config (`config.toml.sample` for metis CLI; `config.yaml.sample` for metis-server) to `config.toml` or `config.yaml` respectively for overrides. Dockerfiles live in `images/`; automation scripts are in `scripts/`.

## Build, Test, and Development Commands
- `cargo check --workspace` quickly verifies the entire workspace compiles.
- `cargo build --workspace --all-targets` produces debug binaries; add `--release` when publishing images.
- `cargo run -p metis -- jobs list` runs the CLI against a server; substitute other subcommands from `metis/src/command`.
- `METIS_CONFIG=metis-server/config.yaml cargo run -p metis-server` launches the HTTP service with the desired config.
- `./scripts/docker-build.sh` builds all deployment containers.

## Documentation Guidelines
- Do not add CLI command details to `README.md` unless explicitly requested; the README has tight space and should stay focused on top-level orientation, so keep command-specific docs elsewhere.

## Coding Style & Naming Conventions
Run `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings` before submitting. Modules and files use snake_case; types and traits use UpperCamelCase; constants are SCREAMING_SNAKE_CASE. Keep each CLI subcommand isolated per file under `metis/src/command` and prefer thin synchronous wrappers around async helpers. Document only non-obvious public behavior with `///` comments.
- Use the `MetisId` type alias for all Metis identifiers instead of raw `String` values.
- CLI git operations should use libgit2; do not shell out to the git binary.
- When a CLI command needs environment variables, declare them on the arg struct (e.g., `#[arg(env = ...)]`) and read them from the parsed args rather than calling `env::var` inside the implementation.

## Testing Guidelines
Run `cargo test --workspace` before opening a pull request. Keep tests near their code (shared helpers belong in `metis-common/src/lib.rs`). For async code use `#[tokio::test]` and descriptive names such as `logs_returns_latest_chunks`. Add regression tests for every fix and cover new branches, especially job-state transitions and Kubernetes interactions.

## Final Task Checklist
Before finishing any task, you **must** run and fix all issues from these commands:
- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

## Commit & Pull Request Guidelines
Please use proper capitalization and sentences. Keep pull request descriptions short and to the point: provide motivation / context for the change, explain what changed, and explain how it was tested.
Please explicitly call out anything that may be confusing or design questions where you made an explicit
choice with tradeoffs, and what the alternatives were. Attach screenshots or CLI snippets for UX changes and highlight configuration, migration, or security impacts.
- **Do not commit screenshots or other images to the git repository.** Instead, upload them to the metis document store under the `screenshots/` directory.

## Configuration & Security Notes
Never commit secrets. Use the sample config files (`config.toml.sample` or `config.yaml.sample`) as templates and load them via `METIS_CONFIG` or env vars such as `OPENAI_API_KEY`. Confirm Docker images reference the intended worker image and namespace before publishing. Add new external integrations to `metis-common` so sensitive values stay centralized and masked.
