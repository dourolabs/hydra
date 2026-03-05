import { Hono } from "hono";
import type { ListSecretsResponse, SetSecretRequest } from "@metis/api";
import { DEV_USERNAME } from "../auth.js";

const ALLOWED_SECRET_NAMES = [
  "OPENAI_API_KEY",
  "ANTHROPIC_API_KEY",
  "CLAUDE_CODE_OAUTH_TOKEN",
  "GITHUB_TOKEN",
  "GITHUB_REFRESH_TOKEN",
];

// In-memory store of configured secret names per user
const userSecrets = new Map<string, Set<string>>();

function getSecrets(username: string): Set<string> {
  let secrets = userSecrets.get(username);
  if (!secrets) {
    secrets = new Set();
    userSecrets.set(username, secrets);
  }
  return secrets;
}

function resolveUsername(raw: string): string {
  return raw === "me" ? DEV_USERNAME : raw;
}

export function resetSecrets(): void {
  userSecrets.clear();
}

export function createSecretRoutes(): Hono {
  const app = new Hono();

  // GET /v1/users/:username/secrets
  app.get("/v1/users/:username/secrets", (c) => {
    const username = resolveUsername(c.req.param("username"));
    const secrets = getSecrets(username);
    const resp: ListSecretsResponse = { secrets: Array.from(secrets) };
    return c.json(resp);
  });

  // PUT /v1/users/:username/secrets/:name
  app.put("/v1/users/:username/secrets/:name", async (c) => {
    const username = resolveUsername(c.req.param("username"));
    const name = c.req.param("name");

    if (!ALLOWED_SECRET_NAMES.includes(name)) {
      return c.json(
        { error: `unknown secret name '${name}'; allowed names: ${ALLOWED_SECRET_NAMES.join(", ")}` },
        400,
      );
    }

    await c.req.json<SetSecretRequest>();
    const secrets = getSecrets(username);
    secrets.add(name);
    return c.json(null);
  });

  // DELETE /v1/users/:username/secrets/:name
  app.delete("/v1/users/:username/secrets/:name", (c) => {
    const username = resolveUsername(c.req.param("username"));
    const name = c.req.param("name");

    if (!ALLOWED_SECRET_NAMES.includes(name)) {
      return c.json(
        { error: `unknown secret name '${name}'; allowed names: ${ALLOWED_SECRET_NAMES.join(", ")}` },
        400,
      );
    }

    const secrets = getSecrets(username);
    secrets.delete(name);
    return c.json(null);
  });

  return app;
}
