# dourolabs/metis repo index

## High-level structure
- Workspace root: Rust workspace with CLI + server + shared models.
- `metis/`: CLI crate (commands under `metis/src/command/`).
- `metis-server/`: Axum API + background workers + Kubernetes orchestration.
- `metis-common/`: Shared types (IDs, API models, job/issue/patch types).
- `metis-ui/`: Web UI.
- `metis-component-library/`: UI component library.
- `metis-s3/`: S3-related utilities/service.
- `metis-build-cache/`: Build cache tooling.
- `images/`: Dockerfiles for server/worker images.
- `scripts/`: automation scripts (docker builds, cluster tooling, etc.).

## Key docs
- `AGENTS.md`: repo-wide workflow/coding/testing requirements.
- `metis-server/AGENTS.md`: route/background layout expectations.
- `README.md`: overall product overview, build/run, config, local dev.
- `DESIGN.md`: system design, issue lifecycle, agent workflow.
- `GETTING_STARTED.md`: onboarding steps and quickstart guidance.

## Review/Comment structs (pointers)
- `metis-common/src/models/reviews.rs`: `ReviewDraft`, `ReviewCommentDraft` (comment metadata, IDs, line ranges).
- `metis-common/src/api/v1/patches.rs`: API-facing `Review` model.
- `metis-server/src/domain/patches.rs`: domain `Review` struct.
- `metis/src/command/issues.rs`: `ReviewSummary`, `ReviewSnapshot` (CLI formatting helpers).

## patches.rs hotspots
- `metis/src/command/patches.rs`: CLI patch subcommands.
- `metis-server/src/routes/patches.rs`: HTTP routes for patches.
- `metis-server/src/domain/patches.rs`: patch domain models/state.
- `metis-server/src/background/poll_github_patches.rs`: GitHub sync logic for patches/reviews/comments.
- `metis-common/src/api/v1/patches.rs`: shared API models for patches.
- `metis-server/src/test/patches.rs`: patch route/domain tests.
