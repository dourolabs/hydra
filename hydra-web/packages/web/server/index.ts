import { serve } from "@hono/node-server";
import { Hono } from "hono";
import { serveStatic } from "@hono/node-server/serve-static";
import { auth } from "./auth.js";
import { sse } from "./sse.js";
import { proxy } from "./proxy.js";
import { config } from "./config.js";
import { logger } from "./logger.js";

const app = new Hono();

// Request logging middleware
app.use("*", async (c, next) => {
  const start = Date.now();
  await next();
  const duration_ms = Date.now() - start;
  const method = c.req.method;
  const path = c.req.path;
  const status = c.res.status;
  logger.info("request", { method, path, status, duration_ms });
});

// Health check
app.get("/health", (c) => c.json({ status: "ok" }));

// Auth routes
app.route("/auth", auth);

// SSE relay: /api/v1/events -> hydra-server /v1/events (before generic proxy)
app.route("/api/v1", sse);

// API proxy: /api/v1/* -> hydra-server /v1/*
app.route("/api/v1", proxy);

// Serve static assets from the Vite build output in production
app.use(
  "/*",
  serveStatic({
    root: "./dist",
  }),
);

// SPA fallback: serve index.html for any unmatched routes
app.use(
  "/*",
  serveStatic({
    root: "./dist",
    rewriteRequestPath: () => "/index.html",
  }),
);

serve({ fetch: app.fetch, port: config.port }, (info) => {
  logger.info("server started", { port: info.port });
});
