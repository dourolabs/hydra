// BigInt cannot be serialized by JSON.stringify by default.
// The wire format uses regular numbers for these fields.
(BigInt.prototype as unknown as { toJSON: () => number }).toJSON = function () {
  return Number(this);
};

import { Hono } from "hono";
import { serve } from "@hono/node-server";
import { Store, StoreError } from "./store.js";
import { authMiddleware } from "./auth.js";
import { createAuthRoutes } from "./routes/auth.js";
import { createIssueRoutes } from "./routes/issues.js";
import { createJobRoutes } from "./routes/jobs.js";
import { createPatchRoutes } from "./routes/patches.js";
import { createDocumentRoutes } from "./routes/documents.js";
import { createRepositoryRoutes } from "./routes/repositories.js";
import { createAgentRoutes } from "./routes/agents.js";
import { createMergeQueueRoutes } from "./routes/merge-queues.js";
import { createNotificationRoutes } from "./routes/notifications.js";
import { createEventRoutes } from "./routes/events.js";
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

// Login endpoint does not require auth
app.route("", createAuthRoutes());

// Auth middleware for all /v1/* routes except /v1/login
app.use("/v1/*", async (c, next) => {
  // Skip auth for login
  if (c.req.path === "/v1/login" && c.req.method === "POST") {
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
  return c.json({ ok: true });
});

// Mount routes
app.route("", createIssueRoutes(store));
app.route("", createJobRoutes(store));
app.route("", createPatchRoutes(store));
app.route("", createDocumentRoutes(store));
app.route("", createRepositoryRoutes(store));
app.route("", createAgentRoutes(store));
app.route("", createMergeQueueRoutes());
app.route("", createNotificationRoutes(store));
app.route("", createEventRoutes(store));

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
  console.log(`@metis/mock-server starting on port ${port}`);
  startMockServer({ port }).then(({ port: resolvedPort }) => {
    console.log(`@metis/mock-server listening on http://localhost:${resolvedPort}`);
  });
}

export { app, store };
