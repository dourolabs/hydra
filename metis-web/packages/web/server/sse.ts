import { Hono } from "hono";
import { getCookie } from "hono/cookie";
import { stream } from "hono/streaming";
import { config } from "./config.js";
import { logger } from "./logger.js";

export const sse = new Hono();

/**
 * SSE relay: GET /events -> metis-server GET /v1/events
 *
 * Dedicated handler for long-lived SSE connections that:
 * - Sets correct SSE headers (Content-Type, Cache-Control, Connection)
 * - Forwards Last-Event-ID from the browser to upstream for reconnection
 * - Injects the auth token from the HttpOnly cookie
 */
sse.get("/events", async (c) => {
  const token = getCookie(c, config.cookieName);
  if (!token) {
    logger.debug("sse: missing auth cookie");
    return c.json({ error: "not authenticated" }, 401);
  }

  const url = new URL(c.req.url);
  const targetUrl = `${config.metisServerUrl}/v1/events${url.search}`;

  const headers: Record<string, string> = {
    Authorization: `Bearer ${token}`,
    Accept: "text/event-stream",
  };

  // Forward Last-Event-ID for reconnection support
  const lastEventId =
    c.req.header("Last-Event-ID") ?? c.req.header("last-event-id");
  if (lastEventId) {
    headers["Last-Event-ID"] = lastEventId;
  }

  let upstreamResp: Response;
  try {
    upstreamResp = await fetch(targetUrl, { headers });
  } catch (err) {
    logger.error("sse: upstream fetch failed", {
      url: targetUrl,
      error: err instanceof Error ? err.message : String(err),
    });
    return c.json({ error: "upstream unreachable" }, 502);
  }

  if (!upstreamResp.ok) {
    logger.error("sse: upstream non-2xx response", {
      url: targetUrl,
      status: upstreamResp.status,
    });
    return c.json(
      { error: `upstream error: ${upstreamResp.status}` },
      upstreamResp.status as 500,
    );
  }

  if (!upstreamResp.body) {
    logger.error("sse: upstream returned no body", { url: targetUrl });
    return c.json({ error: "no upstream body" }, 502);
  }

  logger.debug("sse: connection established", { url: targetUrl });

  c.header("Content-Type", "text/event-stream");
  c.header("Cache-Control", "no-cache");
  c.header("Connection", "keep-alive");
  c.header("X-Accel-Buffering", "no");

  const reader = upstreamResp.body.getReader();

  return stream(c, async (s) => {
    try {
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        await s.write(value);
      }
    } catch {
      // Client disconnected or upstream closed — nothing to do
    } finally {
      reader.releaseLock();
    }
  });
});
