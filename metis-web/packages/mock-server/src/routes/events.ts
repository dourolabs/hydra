import { Hono } from "hono";
import { streamSSE } from "hono/streaming";
import type { Store, StoreEvent } from "../store.js";

export function createEventRoutes(store: Store): Hono {
  const app = new Hono();

  // GET /v1/events (SSE)
  app.get("/v1/events", (c) => {
    const typesFilter = c.req.query("types")?.split(",").filter(Boolean);
    const issueIdsFilter = c.req.query("issue_ids")?.split(",").filter(Boolean);
    const sessionIdsFilter = c.req.query("session_ids")?.split(",").filter(Boolean);
    const patchIdsFilter = c.req.query("patch_ids")?.split(",").filter(Boolean);
    const documentIdsFilter = c.req.query("document_ids")?.split(",").filter(Boolean);

    const lastEventIdHeader = c.req.header("Last-Event-ID");
    const lastEventId = lastEventIdHeader ? Number(lastEventIdHeader) : 0;

    function matchesFilter(event: StoreEvent): boolean {
      if (typesFilter && !typesFilter.includes(event.eventType)) return false;
      if (issueIdsFilter && event.entityType === "issues" && !issueIdsFilter.includes(event.entityId)) return false;
      if (sessionIdsFilter && event.entityType === "sessions" && !sessionIdsFilter.includes(event.entityId)) return false;
      if (patchIdsFilter && event.entityType === "patches" && !patchIdsFilter.includes(event.entityId)) return false;
      if (documentIdsFilter && event.entityType === "documents" && !documentIdsFilter.includes(event.entityId)) return false;
      return true;
    }

    return streamSSE(c, async (stream) => {
      // Send connected event on connect
      await stream.writeSSE({
        event: "connected",
        data: JSON.stringify({ current_seq: store.getCurrentSeq() }),
        id: String(store.getCurrentSeq()),
      });

      // Replay missed events if reconnecting
      if (lastEventId > 0) {
        const missed = store.getEventsSince(lastEventId);
        for (const event of missed) {
          if (!matchesFilter(event)) continue;
          await stream.writeSSE({
            event: event.eventType,
            data: JSON.stringify({
              entity_type: event.entityType,
              entity_id: event.entityId,
              version: event.version,
              timestamp: event.timestamp,
              entity: event.entity,
            }),
            id: String(event.id),
          });
        }
      }

      // Listen for new events
      let closed = false;
      const unsubscribe = store.subscribe(async (event: StoreEvent) => {
        if (closed) return;
        if (!matchesFilter(event)) return;
        try {
          await stream.writeSSE({
            event: event.eventType,
            data: JSON.stringify({
              entity_type: event.entityType,
              entity_id: event.entityId,
              version: event.version,
              timestamp: event.timestamp,
              entity: event.entity,
            }),
            id: String(event.id),
          });
        } catch {
          // Client disconnected
          closed = true;
        }
      });

      // Heartbeat every 15s
      const heartbeatInterval = setInterval(async () => {
        if (closed) {
          clearInterval(heartbeatInterval);
          return;
        }
        try {
          await stream.writeSSE({
            event: "heartbeat",
            data: JSON.stringify({ server_time: new Date().toISOString() }),
            id: String(store.getCurrentSeq()),
          });
        } catch {
          closed = true;
          clearInterval(heartbeatInterval);
        }
      }, 15000);

      // Keep connection open until client disconnects
      stream.onAbort(() => {
        closed = true;
        clearInterval(heartbeatInterval);
        unsubscribe();
      });

      // Keep the stream alive by waiting
      while (!closed) {
        await new Promise((resolve) => setTimeout(resolve, 1000));
      }
    });
  });

  return app;
}
