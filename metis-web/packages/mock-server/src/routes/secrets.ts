import { Hono } from "hono";
import type { ListSecretsResponse, SetSecretRequest } from "@metis/api";

const SECRET_NAME_PATTERN = /^[A-Z][A-Z0-9_]{0,127}$/;

function validateSecretName(name: string): string | null {
  if (!SECRET_NAME_PATTERN.test(name)) {
    return "secret name must be 1-128 chars, start with uppercase letter, and contain only uppercase letters, digits, and underscores";
  }
  if (name.startsWith("METIS_")) {
    return "secret name must not start with METIS_ (reserved prefix)";
  }
  return null;
}

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

export function resetSecrets(): void {
  userSecrets.clear();
}

export function createSecretRoutes(): Hono {
  const app = new Hono();

  // GET /v1/users/:username/secrets
  app.get("/v1/users/:username/secrets", (c) => {
    const username = c.req.param("username");
    const secrets = getSecrets(username);
    const resp: ListSecretsResponse = { secrets: Array.from(secrets) };
    return c.json(resp);
  });

  // PUT /v1/users/:username/secrets/:name
  app.put("/v1/users/:username/secrets/:name", async (c) => {
    const username = c.req.param("username");
    const name = c.req.param("name");

    const validationError = validateSecretName(name);
    if (validationError) {
      return c.json(
        { error: `invalid secret name '${name}': ${validationError}` },
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
    const username = c.req.param("username");
    const name = c.req.param("name");

    const validationError = validateSecretName(name);
    if (validationError) {
      return c.json(
        { error: `invalid secret name '${name}': ${validationError}` },
        400,
      );
    }

    const secrets = getSecrets(username);
    secrets.delete(name);
    return c.json(null);
  });

  return app;
}
