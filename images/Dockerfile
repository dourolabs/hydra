FROM ubuntu:22.04

# Allow callers to inject the Codex API key at build time.
ARG OPENAI_API_KEY

ENV DEBIAN_FRONTEND=noninteractive \
    APP_HOME=/opt/app \
    CODEX_CONFIG_PATH=/etc/codex \
    HOMEBREW_NO_AUTO_UPDATE=1 \
    HOMEBREW_NO_INSTALL_CLEANUP=1 \
    NONINTERACTIVE=1 \
    PATH="/home/linuxbrew/.linuxbrew/bin:/home/linuxbrew/.linuxbrew/sbin:${PATH}" \
    OPENAI_API_KEY=${OPENAI_API_KEY}

# Install Homebrew for Linux and use it to install the Codex CLI as recommended.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        file \
        git \
    && rm -rf /var/lib/apt/lists/* \
    && /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)" \
    && echo 'eval "$(/home/linuxbrew/.linuxbrew/bin/brew shellenv)"' > /etc/profile.d/homebrew.sh \
    && eval "$(/home/linuxbrew/.linuxbrew/bin/brew shellenv)" \
    && brew install codex

WORKDIR ${APP_HOME}

# Copy the build context (current directory) into the image.
COPY . ${APP_HOME}

# Persist the provided API key for the Codex CLI.
RUN mkdir -p ${CODEX_CONFIG_PATH} \
    && if [ -z "${OPENAI_API_KEY}" ]; then \
        echo "ERROR: OPENAI_API_KEY build argument is required. Pass --build-arg OPENAI_API_KEY=your_key."; \
        exit 1; \
    fi \
    && printf "%s\n" "${OPENAI_API_KEY}" > ${CODEX_CONFIG_PATH}/api_key \
    && chmod 600 ${CODEX_CONFIG_PATH}/api_key

# Default to an interactive shell so users can run Codex CLI commands.
CMD ["bash"]
