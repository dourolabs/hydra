import { Hono } from "hono";
import { stream } from "hono/streaming";
import type { Store } from "../store.js";
import { generateId } from "../id.js";
import { DEV_USERNAME } from "../auth.js";
import type {
  Session,
  CreateSessionRequest,
  CreateSessionResponse,
  SessionVersionRecord,
  ListSessionsResponse,
  ListSessionVersionsResponse,
  SessionSummaryRecord,
  SessionSummary,
  KillSessionResponse,
  SessionStatusUpdate,
  SetSessionStatusResponse,
  WorkerContext,
  Status,
} from "@metis/api";

const COLLECTION = "sessions";
const SSE_PREFIX = "session";

function toVersionRecord(
  sessionId: string,
  version: number,
  timestamp: string,
  task: Session,
): SessionVersionRecord {
  return {
    session_id: sessionId,
    version: BigInt(version),
    timestamp,
    session: task,
  };
}

function toSummaryRecord(
  sessionId: string,
  version: number,
  timestamp: string,
  task: Session,
): SessionSummaryRecord {
  const summary: SessionSummary = {
    prompt: task.prompt.slice(0, 100),
    spawned_from: task.spawned_from,
    creator: task.creator,
    status: task.status,
    error: task.error,
    deleted: task.deleted,
    creation_time: task.creation_time,
    start_time: task.start_time,
    end_time: task.end_time,
  };
  return {
    session_id: sessionId,
    version: BigInt(version),
    timestamp,
    session: summary,
  };
}

export function createSessionRoutes(store: Store): Hono {
  const app = new Hono();

  // POST /v1/sessions
  app.post("/v1/sessions", async (c) => {
    const body = await c.req.json<CreateSessionRequest>();
    const id = generateId("session");
    const now = new Date().toISOString();
    const task: Session = {
      prompt: body.prompt,
      context: body.context,
      spawned_from: body.issue_id,
      creator: DEV_USERNAME,
      image: body.image,
      env_vars: body.variables,
      status: "pending" as Status,
      creation_time: now,
    };
    store.create<Session>(COLLECTION, id, task, SSE_PREFIX);
    const resp: CreateSessionResponse = { session_id: id };
    return c.json(resp, 201);
  });

  // GET /v1/sessions
  app.get("/v1/sessions", (c) => {
    const includeDeleted = c.req.query("include_deleted") === "true";
    const q = c.req.query("q");
    const spawnedFrom = c.req.query("spawned_from");
    const status = c.req.query("status");

    const items = store.list<Session>(COLLECTION, includeDeleted);

    let filtered = items;
    if (q) {
      const lower = q.toLowerCase();
      filtered = filtered.filter(({ entry }) =>
        entry.data.prompt.toLowerCase().includes(lower),
      );
    }
    if (spawnedFrom) {
      filtered = filtered.filter(({ entry }) => entry.data.spawned_from === spawnedFrom);
    }
    if (status) {
      const statuses = new Set(status.split(","));
      filtered = filtered.filter(({ entry }) => statuses.has(entry.data.status));
    }

    const sessions: SessionSummaryRecord[] = filtered.map(({ id, entry }) =>
      toSummaryRecord(id, entry.version, entry.timestamp, entry.data),
    );
    const resp: ListSessionsResponse = { sessions };
    return c.json(resp);
  });

  // GET /v1/sessions/:id
  app.get("/v1/sessions/:id", (c) => {
    const id = c.req.param("id");
    const entry = store.get<Session>(COLLECTION, id);
    if (!entry) {
      return c.json({ error: `session '${id}' not found` }, 404);
    }
    return c.json(toVersionRecord(id, entry.version, entry.timestamp, entry.data));
  });

  // GET /v1/sessions/:id/versions/:version
  app.get("/v1/sessions/:id/versions/:version", (c) => {
    const id = c.req.param("id");
    const version = Number(c.req.param("version"));
    const entry = store.getVersion<Session>(COLLECTION, id, version);
    if (!entry) {
      return c.json({ error: `session '${id}' version ${version} not found` }, 404);
    }
    return c.json(toVersionRecord(id, entry.version, entry.timestamp, entry.data));
  });

  // DELETE /v1/sessions/:id — kill session
  // The real server sends a kill signal to K8s but the session stays "running"
  // until the pod actually terminates. We simulate this by not updating
  // the store immediately, so refetches still return "running".
  app.delete("/v1/sessions/:id", (c) => {
    const id = c.req.param("id");
    const entry = store.get<Session>(COLLECTION, id);
    if (!entry) {
      return c.json({ error: `session '${id}' not found` }, 404);
    }
    const resp: KillSessionResponse = { session_id: id, status: "failed" };
    return c.json(resp);
  });

  // GET /v1/sessions/:id/logs
  app.get("/v1/sessions/:id/logs", (c) => {
    const id = c.req.param("id");
    const entry = store.get<Session>(COLLECTION, id);
    if (!entry) {
      return c.json({ error: `session '${id}' not found` }, 404);
    }
    const watch = c.req.query("watch") === "true";
    if (watch) {
      return stream(c, async (s) => {
        c.header("Content-Type", "text/event-stream");
        c.header("Cache-Control", "no-cache");
        c.header("Connection", "keep-alive");
        await s.write(`data: [mock] Session ${id} log line 1\n\n`);
        await s.write(`data: [mock] Session ${id} log line 2\n\n`);
        await s.write(`data: [mock] Session ${id} complete\n\n`);
      });
    }
    return c.text(`[mock] Session ${id} log output\n[mock] Session completed successfully\n`);
  });

  // POST /v1/sessions/:id/status
  app.post("/v1/sessions/:id/status", async (c) => {
    const id = c.req.param("id");
    const body = await c.req.json<SessionStatusUpdate>();
    const entry = store.get<Session>(COLLECTION, id);
    if (!entry) {
      return c.json({ error: `session '${id}' not found` }, 404);
    }
    let newStatus: Status;
    const updates: Partial<Session> = {};
    if (body.status === "complete") {
      newStatus = "complete";
      updates.last_message = body.last_message;
      updates.end_time = new Date().toISOString();
    } else if (body.status === "failed") {
      newStatus = "failed";
      updates.error = { job_engine_error: { reason: body.reason } };
      updates.end_time = new Date().toISOString();
    } else {
      newStatus = "unknown";
    }
    const updated: Session = { ...entry.data, ...updates, status: newStatus };
    store.update<Session>(COLLECTION, id, updated, SSE_PREFIX);
    const resp: SetSessionStatusResponse = { session_id: id, status: newStatus };
    return c.json(resp);
  });

  // GET /v1/sessions/:id/context
  app.get("/v1/sessions/:id/context", (c) => {
    const id = c.req.param("id");
    const entry = store.get<Session>(COLLECTION, id);
    if (!entry) {
      return c.json({ error: `session '${id}' not found` }, 404);
    }
    const task = entry.data;
    const resp: WorkerContext = {
      request_context: task.context.type === "git_repository"
        ? { type: "git_repository", url: task.context.url, rev: task.context.rev }
        : { type: "none" },
      prompt: task.prompt,
      model: task.model,
      variables: task.env_vars ?? {},
    };
    return c.json(resp);
  });

  // GET /v1/sessions/:id/versions
  app.get("/v1/sessions/:id/versions", (c) => {
    const id = c.req.param("id");
    const allVersions = store.listVersions<Session>(COLLECTION, id);
    if (allVersions.length === 0) {
      return c.json({ error: `session '${id}' not found` }, 404);
    }
    const versions = allVersions.map((v) =>
      toVersionRecord(id, v.version, v.timestamp, v.data),
    );
    const resp: ListSessionVersionsResponse = { versions };
    return c.json(resp);
  });

  return app;
}
