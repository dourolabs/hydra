FROM rust:1.68.1 as planner
RUN cargo install cargo-chef

WORKDIR /app
# Copy the whole project
COPY . .
# Prepare a build plan ("recipe")
RUN cargo chef prepare --recipe-path recipe.json

FROM rust:1.68.1 AS builder
RUN cargo install cargo-chef

# Copy the build plan from the previous Docker stage
COPY --from=planner /app/recipe.json recipe.json

# Build dependencies - this layer is cached as long as `recipe.json`
# doesn't change.
RUN cargo chef cook --recipe-path recipe.json

# Build the whole project
COPY . .

RUN cargo build

# ENTRYPOINT ["./scripts/codex-entrypoint.sh"]

# Default to an interactive shell so users can run Codex CLI commands.
# CMD ["bash"]
