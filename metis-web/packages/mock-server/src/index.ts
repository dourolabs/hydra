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
import { createEventRoutes } from "./routes/events.js";

const store = new Store();
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

// Mount routes
app.route("", createIssueRoutes(store));
app.route("", createJobRoutes(store));
app.route("", createPatchRoutes(store));
app.route("", createDocumentRoutes(store));
app.route("", createRepositoryRoutes(store));
app.route("", createAgentRoutes(store));
app.route("", createMergeQueueRoutes());
app.route("", createEventRoutes(store));

const port = Number(process.env.PORT ?? 8080);

console.log(`@metis/mock-server starting on port ${port}`);
serve({ fetch: app.fetch, port }, (info) => {
  console.log(`@metis/mock-server listening on http://localhost:${info.port}`);
});

export { app, store };
