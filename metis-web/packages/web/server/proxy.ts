import { Hono } from "hono";
import { getCookie } from "hono/cookie";
import { config } from "./config.js";

export const proxy = new Hono();

/**
 * API proxy: /api/v1/* -> metis-server /v1/*
 * Forwards the auth token from the cookie as an Authorization: Bearer header.
 */
proxy.all("/*", async (c) => {
  const token = getCookie(c, config.cookieName);

  if (!token) {
    return c.json({ error: "not authenticated" }, 401);
  }

  // Strip the leading /api prefix to get the metis-server path
  const url = new URL(c.req.url);
  const targetPath = url.pathname.replace(/^\/api/, "");
  const targetUrl = `${config.metisServerUrl}${targetPath}${url.search}`;

  const headers = new Headers(c.req.raw.headers);
  headers.set("Authorization", `Bearer ${token}`);
  // Remove cookie header to avoid leaking cookies to upstream
  headers.delete("cookie");
  // Remove host header so the upstream sees its own host
  headers.delete("host");

  const resp = await fetch(targetUrl, {
    method: c.req.method,
    headers,
    body: c.req.method !== "GET" && c.req.method !== "HEAD" ? c.req.raw.body : undefined,
    // @ts-expect-error -- Node.js fetch supports duplex for streaming request bodies
    duplex: c.req.method !== "GET" && c.req.method !== "HEAD" ? "half" : undefined,
  });

  return new Response(resp.body, {
    status: resp.status,
    statusText: resp.statusText,
    headers: resp.headers,
  });
});
