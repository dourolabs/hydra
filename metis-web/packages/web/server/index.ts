import { serve } from "@hono/node-server";
import { Hono } from "hono";
import { serveStatic } from "@hono/node-server/serve-static";
import { auth } from "./auth.js";
import { proxy } from "./proxy.js";
import { config } from "./config.js";

const app = new Hono();

// Health check
app.get("/health", (c) => c.json({ status: "ok" }));

// Auth routes
app.route("/auth", auth);

// API proxy: /api/v1/* -> metis-server /v1/*
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
  console.log(`BFF server listening on http://localhost:${info.port}`);
});
