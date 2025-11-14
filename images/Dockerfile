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

# set ENTRYPOINT for reloading nvm-environment
ENTRYPOINT ["bash", "-c", "source $NVM_DIR/nvm.sh && exec \"$@\"", "--"]

WORKDIR ${APP_HOME}

# Copy the build context (current directory) into the image.
COPY . ${APP_HOME}

ENTRYPOINT ["./scripts/codex-entrypoint.sh"]

# Default to an interactive shell so users can run Codex CLI commands.
CMD ["bash"]
