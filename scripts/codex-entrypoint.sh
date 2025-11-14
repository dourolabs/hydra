#!/usr/bin/env bash

source $NVM_DIR/nvm.sh

printenv OPENAI_API_KEY | codex login --with-api-key

exec "$@"
