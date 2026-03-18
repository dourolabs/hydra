# Scenario: Server Initialization and Dashboard Load

**ID:** server-init
**Category:** core
**Priority:** P0
**Prerequisites:** Hydra binary built (`cargo build -p hydra`), `CLAUDE_CODE_OAUTH_TOKEN` and `GH_TOKEN` environment variables set
**Estimated duration:** 3 minutes

## Description

Verify that the Hydra server initializes successfully using the `--config` flag (non-interactive mode) and that the dashboard loads at `http://localhost:8080`. This is the foundational scenario that all other scenarios depend on.

## Steps (via dashboard)

1. Prepare a config file for non-interactive server init:
   ```yaml
   hydra:
     namespace: "test"
     server_hostname: "127.0.0.1:8080"
     CLAUDE_CODE_OAUTH_TOKEN: "${CLAUDE_CODE_OAUTH_TOKEN}"
   storage_backend: "sqlite"
   sqlite_path: "~/.hydra/server/hydra.db"
   job_engine: "local"
   auth_mode: "local"
   github_token: "${GH_TOKEN}"
   username: "test-agent"
   job:
     default_model: "opus"
   ```
2. Run `hydra server init --config <path-to-config>` to start the server
3. Wait for the server health check to pass (GET `http://localhost:8080/health`)
4. Navigate to `http://localhost:8080` using Playwright MCP
5. Verify the dashboard loads by checking for the presence of the main navigation elements
6. Take an accessibility snapshot of the landing page to confirm key UI elements render

## Expected Results

- The server starts without interactive prompts
- The health check endpoint returns a success response
- The dashboard loads at `http://localhost:8080` with no errors
- The main navigation is visible, including links to Issues, Patches, Repos, Documents, and Agents
- No JavaScript errors or broken page elements are present
