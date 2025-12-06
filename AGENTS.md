# Repository Guidelines

## Project Structure & Module Organization
Workspace crates: `metis` (CLI), `metis-server` (Axum API and background workers), and `metis-common` (shared models). CLI subcommands stay under `metis/src/command`, while routes, job engines, and the in-memory store live in `metis-server/src`. Copy each `config.toml.sample` to `config.toml` for overrides. Dockerfiles live in `images/`; automation scripts are in `scripts/`.

## Build, Test, and Development Commands
- `cargo check --workspace` quickly verifies the entire workspace compiles.
- `cargo build --workspace --all-targets` produces debug binaries; add `--release` when publishing images.
- `cargo run -p metis -- jobs list` runs the CLI against a server; substitute other subcommands from `metis/src/command`.
- `METIS_CONFIG=metis-server/config.toml cargo run -p metis-server` launches the HTTP service with the desired config.
- `./scripts/docker-build.sh` builds all deployment containers.

## Coding Style & Naming Conventions
Run `cargo fmt --all` and `cargo clippy --workspace --all-targets -- -D warnings` before submitting. Modules and files use snake_case; types and traits use UpperCamelCase; constants are SCREAMING_SNAKE_CASE. Keep each CLI subcommand isolated per file under `metis/src/command` and prefer thin synchronous wrappers around async helpers. Document only non-obvious public behavior with `///` comments.

## Testing Guidelines
Run `cargo test --workspace` before opening a pull request. Keep tests near their code (routes under `metis-server/src/routes`, shared helpers in `metis-common/src/lib.rs`). For async code use `#[tokio::test]` and descriptive names such as `logs_returns_latest_chunks`. Add regression tests for every fix and cover new branches, especially job-state transitions and Kubernetes interactions.

## Commit & Pull Request Guidelines
Recent history shows short, lower-case, imperative commits (e.g., `fix dockerfile`). Follow that style and keep changes scoped. Each pull request should explain the motivation, outline functional changes, link issues, and include test evidence. Attach screenshots or CLI snippets for UX changes and highlight configuration, migration, or security impacts.

## Configuration & Security Notes
Never commit secrets. Use the `config.toml.sample` files as templates and load them via `METIS_CONFIG` or env vars such as `OPENAI_API_KEY`. Confirm Docker images reference the intended worker image and namespace before publishing. Add new external integrations to `metis-common` so sensitive values stay centralized and masked.
