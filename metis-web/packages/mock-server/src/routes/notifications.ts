import { Hono } from "hono";
import type { Store } from "../store.js";
import type {
  Notification,
  NotificationResponse,
  ListNotificationsResponse,
  MarkReadResponse,
} from "@metis/api";

const COLLECTION = "notifications";

function toResponse(id: string, notification: Notification): NotificationResponse {
  return { notification_id: id, notification };
}

export function createNotificationRoutes(store: Store): Hono {
  const app = new Hono();

  // GET /v1/notifications
  app.get("/v1/notifications", (c) => {
    const isReadParam = c.req.query("is_read");
    const items = store.list<Notification>(COLLECTION);

    let filtered = items;
    if (isReadParam !== undefined && isReadParam !== "") {
      const isRead = isReadParam === "true";
      filtered = filtered.filter(({ entry }) => entry.data.is_read === isRead);
    }

    const notifications: NotificationResponse[] = filtered.map(({ id, entry }) =>
      toResponse(id, entry.data),
    );
    const resp: ListNotificationsResponse = { notifications };
    return c.json(resp);
  });

  // POST /v1/notifications/:notificationId/read
  app.post("/v1/notifications/:notificationId/read", (c) => {
    const id = c.req.param("notificationId");
    const entry = store.get<Notification>(COLLECTION, id);
    if (!entry) {
      return c.json({ error: `notification '${id}' not found` }, 404);
    }
    if (!entry.data.is_read) {
      store.update<Notification>(COLLECTION, id, { ...entry.data, is_read: true }, null);
    }
    const resp: MarkReadResponse = { marked: 1n };
    return c.json(resp);
  });

  // POST /v1/notifications/read-all
  app.post("/v1/notifications/read-all", (c) => {
    const items = store.list<Notification>(COLLECTION);
    let marked = 0n;
    for (const { id, entry } of items) {
      if (!entry.data.is_read) {
        store.update<Notification>(COLLECTION, id, { ...entry.data, is_read: true }, null);
        marked++;
      }
    }
    const resp: MarkReadResponse = { marked };
    return c.json(resp);
  });

  return app;
}
