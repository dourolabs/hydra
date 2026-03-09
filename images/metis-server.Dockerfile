FROM rust:1.88.0 as planner
RUN cargo install cargo-chef

WORKDIR /app
# Copy the whole project
COPY . .
# Prepare a build plan ("recipe")
RUN cargo chef prepare --recipe-path recipe.json

FROM rust:1.88.0 AS builder
RUN cargo install cargo-chef

WORKDIR /app

# Copy the build plan from the previous Docker stage
COPY --from=planner /app/recipe.json recipe.json

# Build dependencies - this layer is cached as long as `recipe.json`
# doesn't change.
RUN cargo chef cook --recipe-path recipe.json --features postgres

# Build the whole project
COPY . .

RUN cargo build --bin metis-server --release --features postgres

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/target/release/metis-server /usr/local/bin/metis-server

ENV RUST_LOG=info
ENTRYPOINT ["metis-server"]

# Default to an interactive shell so users can run Codex CLI commands.
# CMD ["bash"]
