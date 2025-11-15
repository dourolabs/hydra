FROM rust:1.88.0 as planner
RUN cargo install cargo-chef

WORKDIR /app
# Copy the whole project
COPY . .
# Prepare a build plan ("recipe")
RUN cargo chef prepare --recipe-path recipe.json

FROM rust:1.88.0 AS builder
RUN cargo install cargo-chef

# Copy the build plan from the previous Docker stage
COPY --from=planner /app/recipe.json recipe.json

# Build dependencies - this layer is cached as long as `recipe.json`
# doesn't change.
RUN cargo chef cook --recipe-path recipe.json

# Build the whole project
COPY . .

RUN cargo build

ENV CONFIG_PATH=./metis-server/config.toml

ENTRYPOINT ["./metis-server/target/debug/metis-server"]

# Default to an interactive shell so users can run Codex CLI commands.
# CMD ["bash"]
