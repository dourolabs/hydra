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

FROM rust:1.88.0
ARG NODE_VERSION=22

ENV DEBIAN_FRONTEND=noninteractive \
    APP_HOME=/opt/app \
    CODEX_CONFIG_PATH=/home/worker/.config/codex \
    NONINTERACTIVE=1

# Install prerequisites
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        file \
        git \
        gnupg \
        jq \
        libcairo2 \
        libasound2 \
        libatk-bridge2.0-0 \
        libatk1.0-0 \
        libatspi2.0-0 \
        libexpat1 \
        libfontconfig1 \
        libglib2.0-0 \
        libnspr4 \
        libpango-1.0-0 \
        libpangocairo-1.0-0 \
        libcups2 \
        libdbus-1-3 \
        libdrm2 \
        libx11-6 \
        libgbm1 \
        libgtk-3-0 \
        libnss3 \
        libxcb1 \
        libx11-xcb1 \
        libxcomposite1 \
        libxcursor1 \
        libxdamage1 \
        libxext6 \
        libxfixes3 \
        libxi6 \
        libxrandr2 \
        libxrender1 \
        libxss1 \
        libxtst6 \
        libxkbcommon0 \
        libxshmfence1 \
        libu2f-udev \
        lsb-release \
        nodejs \
        npm \
        fonts-liberation \
        fonts-noto-color-emoji \
        ripgrep \
        wget \
        xdg-utils \
    && curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg | dd of=/usr/share/keyrings/githubcli-archive-keyring.gpg \
    && chmod go+r /usr/share/keyrings/githubcli-archive-keyring.gpg \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" | tee /etc/apt/sources.list.d/github-cli.list > /dev/null \
    && apt-get update \
    && apt-get install -y --no-install-recommends gh \
    && rm -rf /var/lib/apt/lists/*

# Ensure rustfmt and clippy are available for formatting and linting tasks run inside the worker image
RUN rustup component add rustfmt clippy

# Install 1Password CLI
RUN curl -fsSL https://downloads.1password.com/linux/keys/1password.asc | \
    gpg --dearmor -o /usr/share/keyrings/1password-archive-keyring.gpg && \
    echo "deb [arch=amd64 signed-by=/usr/share/keyrings/1password-archive-keyring.gpg] https://downloads.1password.com/linux/debian/amd64 stable main" \
    > /etc/apt/sources.list.d/1password.list && \
    apt-get update && apt-get install -y --no-install-recommends 1password-cli && \
    rm -rf /var/lib/apt/lists/*
RUN op --version

# Create a non-root user
RUN useradd -m -s /bin/bash -u 1000 worker \
    && mkdir -p ${APP_HOME} /usr/local/bin \
    && chown -R worker:worker ${APP_HOME}

# Install nvm and node for the non-root user
USER worker
WORKDIR /home/worker

# Install nvm for the worker user
RUN curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.3/install.sh | bash

# Set NVM environment variable for the worker user
ENV NVM_DIR=/home/worker/.nvm

# Install node, codex, claude, puppeteer, and playwright as the non-root user
RUN bash -c "source $NVM_DIR/nvm.sh && nvm install $NODE_VERSION && npm install -g @openai/codex @anthropic-ai/claude-code puppeteer playwright pnpm"

# Pre-install Playwright's Chromium so E2E tests work without a per-session download
RUN bash -c "source $NVM_DIR/nvm.sh && npx playwright install chromium"

# Create stable symlink to nvm-installed node binaries so they are in PATH
# without requiring `source nvm.sh`
RUN bash -c "source $NVM_DIR/nvm.sh && ln -sf \$(dirname \$(which node)) $NVM_DIR/default"
ENV PATH="/home/worker/.nvm/default:$PATH"

# Switch back to root to copy files and set permissions
USER root

WORKDIR ${APP_HOME}

# Copy the built metis CLI into PATH and make it accessible
COPY --from=builder /app/target/release/metis /usr/local/bin/metis
RUN chmod +x /usr/local/bin/metis

# Ensure the worker user owns the app directory and can write to it
RUN chown -R worker:worker ${APP_HOME}

# Switch to the non-root user
USER worker

# Default to an interactive shell so users can run Codex CLI commands.
CMD ["bash"]
