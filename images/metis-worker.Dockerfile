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
# Build dependencies - this layer is cached as long as `recipe.json` doesn't change.
RUN cargo chef cook --recipe-path recipe.json
# Build the whole project
COPY . .
# Build only the metis CLI
RUN cargo build --bin metis --release

FROM ubuntu:22.04
ARG NODE_VERSION=22

ENV DEBIAN_FRONTEND=noninteractive \
    APP_HOME=/opt/app \
    CODEX_CONFIG_PATH=/etc/codex \
    NONINTERACTIVE=1

# Install prerequisites, add a non-root user, and install Homebrew as that user.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        file \
        git \
        nodejs \
        npm \
    && rm -rf /var/lib/apt/lists/*

# install nvm
RUN curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.3/install.sh | bash

# set env
ENV NVM_DIR=/root/.nvm

# install node
RUN bash -c "source $NVM_DIR/nvm.sh && nvm install $NODE_VERSION && npm install -g @openai/codex"

WORKDIR ${APP_HOME}

COPY ./scripts/worker-entrypoint.sh /usr/local/worker-entrypoint.sh

# Copy the built metis CLI into PATH
COPY --from=builder /app/target/release/metis /usr/local/bin/metis

ENTRYPOINT ["/usr/local/worker-entrypoint.sh"]

# Default to an interactive shell so users can run Codex CLI commands.
CMD ["bash"]
