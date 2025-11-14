FROM ubuntu:22.04

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
    && rm -rf /var/lib/apt/lists/* \
    && npm install -g @openai/codex 


WORKDIR ${APP_HOME}

# Copy the build context (current directory) into the image.
COPY . ${APP_HOME}

# Default to an interactive shell so users can run Codex CLI commands.
CMD ["bash"]
