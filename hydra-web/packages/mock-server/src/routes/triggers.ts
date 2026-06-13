import { Hono } from "hono";
import type { Store } from "../store.js";
import { generateId } from "../id.js";
import type {
  Trigger,
  TriggerVersionRecord,
  UpsertTriggerRequest,
  UpsertTriggerResponse,
  ListTriggersResponse,
  ListTriggerVersionsResponse,
} from "@hydra/api";

const COLLECTION = "triggers";

function toRequestTrigger(body: UpsertTriggerRequest): Trigger {
  return {
    enabled: body.enabled,
    schedule: body.schedule,
    actions: body.actions,
    creator: body.creator,
  };
}

function toVersionRecord(
  triggerId: string,
  version: number,
  timestamp: string,
  trigger: Trigger,
  creationTime: string,
): TriggerVersionRecord {
  return {
    trigger_id: triggerId,
    version: BigInt(version),
    timestamp,
    trigger,
    creation_time: creationTime,
  };
}

export function createTriggerRoutes(store: Store): Hono {
  const app = new Hono();

  // POST /v1/triggers
  app.post("/v1/triggers", async (c) => {
    const body = await c.req.json<UpsertTriggerRequest>();
    const id = generateId("trigger");
    const trigger = toRequestTrigger(body);
    const entry = store.create<Trigger>(COLLECTION, id, trigger, null);
    const resp: UpsertTriggerResponse = {
      trigger_id: id,
      version: BigInt(entry.version),
    };
    return c.json(resp, 201);
  });

  // PUT /v1/triggers/:id
  app.put("/v1/triggers/:id", async (c) => {
    const id = c.req.param("id");
    const body = await c.req.json<UpsertTriggerRequest>();
    // Mirror the prod behaviour: `last_fired_at` is server-owned and carried
    // forward across updates by `update_trigger`, so preserve whatever the
    // store already has rather than letting upsert clobber it to null.
    const existing = store.get<Trigger>(COLLECTION, id);
    const trigger: Trigger = {
      ...toRequestTrigger(body),
      last_fired_at: existing?.data.last_fired_at ?? null,
    };
    const entry = store.update<Trigger>(COLLECTION, id, trigger, null);
    const resp: UpsertTriggerResponse = {
      trigger_id: id,
      version: BigInt(entry.version),
    };
    return c.json(resp);
  });

  // GET /v1/triggers/:id
  app.get("/v1/triggers/:id", (c) => {
    const id = c.req.param("id");
    const includeDeleted = c.req.query("include_archived") === "true";
    const entry = store.get<Trigger>(COLLECTION, id, includeDeleted);
    if (!entry) {
      return c.json({ error: `trigger '${id}' not found` }, 404);
    }
    const creationTime = store.getCreationTime(COLLECTION, id)!;
    return c.json(
      toVersionRecord(id, entry.version, entry.timestamp, entry.data, creationTime),
    );
  });

  // GET /v1/triggers/:id/versions/:version
  app.get("/v1/triggers/:id/versions/:version", (c) => {
    const id = c.req.param("id");
    const version = Number(c.req.param("version"));
    const entry = store.getVersion<Trigger>(COLLECTION, id, version);
    if (!entry) {
      return c.json({ error: `trigger '${id}' version ${version} not found` }, 404);
    }
    const creationTime = store.getCreationTime(COLLECTION, id)!;
    return c.json(
      toVersionRecord(id, entry.version, entry.timestamp, entry.data, creationTime),
    );
  });

  // GET /v1/triggers
  app.get("/v1/triggers", (c) => {
    const includeDeleted = c.req.query("include_archived") === "true";
    const items = store.list<Trigger>(COLLECTION, includeDeleted);

    // Sort by last-update time descending (most recently updated first)
    items.sort((a, b) => b.entry.timestamp.localeCompare(a.entry.timestamp));

    const triggers: TriggerVersionRecord[] = items.map(({ id, entry }) => {
      const creationTime = store.getCreationTime(COLLECTION, id)!;
      return toVersionRecord(id, entry.version, entry.timestamp, entry.data, creationTime);
    });
    const resp: ListTriggersResponse = { triggers };
    return c.json(resp);
  });

  // GET /v1/triggers/:id/versions
  app.get("/v1/triggers/:id/versions", (c) => {
    const id = c.req.param("id");
    const allVersions = store.listVersions<Trigger>(COLLECTION, id);
    if (allVersions.length === 0) {
      return c.json({ error: `trigger '${id}' not found` }, 404);
    }
    const creationTime = store.getCreationTime(COLLECTION, id)!;
    const versions = allVersions.map((v) =>
      toVersionRecord(id, v.version, v.timestamp, v.data, creationTime),
    );
    const resp: ListTriggerVersionsResponse = { versions };
    return c.json(resp);
  });

  // DELETE /v1/triggers/:id
  app.delete("/v1/triggers/:id", (c) => {
    const id = c.req.param("id");
    const entry = store.delete<Trigger>(COLLECTION, id, null);
    const creationTime = store.getCreationTime(COLLECTION, id)!;
    return c.json(
      toVersionRecord(id, entry.version, entry.timestamp, entry.data, creationTime),
    );
  });

  return app;
}
