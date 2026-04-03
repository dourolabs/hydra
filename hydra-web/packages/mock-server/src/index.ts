// BigInt cannot be serialized by JSON.stringify by default.
// The wire format uses regular numbers for these fields.
(BigInt.prototype as unknown as { toJSON: () => number }).toJSON = function () {
  return Number(this);
};

import { Hono } from "hono";
import { serve } from "@hono/node-server";
import { getCookie } from "hono/cookie";
import { Store, StoreError } from "./store.js";
import { authMiddleware } from "./auth.js";
import { createAuthRoutes, createBffAuthRoutes } from "./routes/auth.js";
import { createIssueRoutes } from "./routes/issues.js";
import { createSessionRoutes } from "./routes/sessions.js";
import { createPatchRoutes } from "./routes/patches.js";
import { createDocumentRoutes } from "./routes/documents.js";
import { createRepositoryRoutes } from "./routes/repositories.js";
import { createAgentRoutes } from "./routes/agents.js";
import { createMergeQueueRoutes } from "./routes/merge-queues.js";
import { createEventRoutes } from "./routes/events.js";
import { createLabelRoutes } from "./routes/labels.js";
import { createRelationRoutes } from "./routes/relations.js";
import { createSecretRoutes, resetSecrets } from "./routes/secrets.js";
import { loadSeedData } from "./seed.js";

const store = new Store();

// Load seed data on startup
loadSeedData(store);
const app = new Hono();

// X-Mock-Error middleware: return simulated error for any request with this header
app.use("*", async (c, next) => {
  const mockError = c.req.header("X-Mock-Error");
  if (mockError) {
    const status = Number(mockError);
    return c.json({ error: "simulated server error" }, status as 400);
  }
  await next();
});

// Health endpoint (no auth required)
app.get("/health", (c) => c.json({ status: "ok" }));

// BFF-style auth routes (cookie-based) for e2e tests
app.route("/auth", createBffAuthRoutes());

// Login endpoint does not require auth
app.route("", createAuthRoutes());

// BFF proxy rewrite: /api/v1/* -> /v1/* with cookie-to-Bearer conversion
app.all("/api/v1/*", async (c) => {
  const token = getCookie(c, "hydra_token");
  // Allow unauthenticated access to the GitHub client-id endpoint
  // (called before user is logged in to determine if GitHub auth is available)
  if (!token && c.req.path !== "/api/v1/github/app/client-id") {
    return c.json({ error: "not authenticated" }, 401);
  }

  const url = new URL(c.req.url);
  const rewrittenPath = url.pathname.replace(/^\/api\/v1/, "/v1");
  const rewrittenUrl = new URL(rewrittenPath + url.search, url.origin);

  const headers = new Headers(c.req.raw.headers);
  headers.set("Authorization", `Bearer ${token}`);
  headers.delete("cookie");

  const newRequest = new Request(rewrittenUrl.toString(), {
    method: c.req.method,
    headers,
    body:
      c.req.method !== "GET" && c.req.method !== "HEAD"
        ? c.req.raw.body
        : undefined,
    // @ts-expect-error -- Node.js fetch supports duplex for streaming request bodies
    duplex:
      c.req.method !== "GET" && c.req.method !== "HEAD" ? "half" : undefined,
  });

  return app.fetch(newRequest);
});

// Auth middleware for all /v1/* routes except /v1/login and /v1/github/app/client-id
app.use("/v1/*", async (c, next) => {
  // Skip auth for login
  if (c.req.path === "/v1/login" && c.req.method === "POST") {
    await next();
    return;
  }
  // Skip auth for GitHub client-id check (called before user is logged in)
  if (c.req.path === "/v1/github/app/client-id" && c.req.method === "GET") {
    await next();
    return;
  }
  return authMiddleware(c, next);
});

// Error handling for StoreError
app.onError((err, c) => {
  if (err instanceof StoreError) {
    return c.json({ error: err.message }, err.status as 400);
  }
  console.error("Unhandled error:", err);
  return c.json({ error: "internal server error" }, 500);
});

// POST /v1/dev/reset — restore store to seed data state
app.post("/v1/dev/reset", (c) => {
  loadSeedData(store);
  resetSecrets();
  return c.json({ ok: true });
});

// Mount routes
app.route("", createIssueRoutes(store));
app.route("", createSessionRoutes(store));
app.route("", createPatchRoutes(store));
app.route("", createDocumentRoutes(store));
app.route("", createRepositoryRoutes(store));
app.route("", createAgentRoutes(store));
app.route("", createLabelRoutes(store));
app.route("", createMergeQueueRoutes());
app.route("", createRelationRoutes(store));
app.route("", createEventRoutes(store));
app.route("", createSecretRoutes());

export interface MockServerHandle {
  port: number;
  close: () => Promise<void>;
}

/**
 * Start the mock server on the given port (default: random).
 * Returns a handle with the resolved port and a close function.
 */
export function startMockServer(options?: { port?: number }): Promise<MockServerHandle> {
  return new Promise((resolve) => {
    const serverPort = options?.port ?? 0;
    const server = serve({ fetch: app.fetch, port: serverPort }, (info) => {
      resolve({
        port: info.port,
        close: () => new Promise<void>((res) => server.close(() => res())),
      });
    });
  });
}

// Auto-start when running as a standalone server (not during tests)
if (!process.env.VITEST) {
  const port = Number(process.env.PORT ?? 8080);
  console.log(`@hydra/mock-server starting on port ${port}`);
  startMockServer({ port }).then(({ port: resolvedPort }) => {
    console.log(`@hydra/mock-server listening on http://localhost:${resolvedPort}`);
  });
}

export { app, store };
