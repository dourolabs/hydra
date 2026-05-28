import { Hono } from "hono";
import { stream } from "hono/streaming";
import type { Store } from "../store.js";
import { generateId } from "../id.js";
import { DEV_USERNAME } from "../auth.js";
import type {
  Session,
  CreateSessionRequest,
  CreateSessionResponse,
  SessionEvent,
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
  MountItem,
} from "@hydra/api";

// `Session` lost its top-level `prompt` / `interactive` / `model` fields in
// Phase D step 13 (PR-2); the equivalents now live on `session.mode` and
// `session.agent_config`. These helpers narrow the discriminated union so
// the mock-server's filtering / summary / context paths keep working
// against the post-PR-2 wire shape. They also fall back to the legacy
// fields when present (seed.json fixtures still use the pre-PR-2 shape;
// the Rust deserializer has the same tolerance via a custom `Deserialize`
// impl).
type LegacySession = Session & {
  prompt?: string;
  interactive?: { conversation_id?: string } | null;
};

function promptOf(session: Session): string {
  const legacy = session as LegacySession;
  if (session.mode?.type === "headless" || session.mode?.type === "interactive") {
    return session.agent_config?.system_prompt ?? "";
  }
  return legacy.prompt ?? "";
}

export function conversationIdOf(session: Session): string | null {
  const legacy = session as LegacySession;
  if (session.mode?.type === "interactive") return session.mode.conversation_id;
  return legacy.interactive?.conversation_id ?? null;
}

const COLLECTION = "sessions";
const SSE_PREFIX = "session";

// Per-session SessionEvent log. Mirrors the conversation event map in
// routes/conversations.ts; the chat read path (Phase C step 11) fans out
// over this on `GET /v1/sessions/:id/events`.
const sessionEvents = new Map<string, SessionEvent[]>();

export function clearSessionEvents(): void {
  sessionEvents.clear();
}

export function setSessionEvents(
  sessionId: string,
  events: SessionEvent[],
): void {
  sessionEvents.set(sessionId, [...events]);
}

export function appendSessionEvent(
  sessionId: string,
  event: SessionEvent,
): void {
  const existing = sessionEvents.get(sessionId);
  if (existing) {
    existing.push(event);
  } else {
    sessionEvents.set(sessionId, [event]);
  }
}

export function getSessionEventsFor(sessionId: string): SessionEvent[] {
  return sessionEvents.get(sessionId) ?? [];
}

function getSessionEvents(sessionId: string): SessionEvent[] {
  return getSessionEventsFor(sessionId);
}

/**
 * List session ids linked to a conversation, ordered oldest-first by
 * creation_time. Mirrors the backend's
 * `list_session_ids_by_conversation_id` shape; used by the
 * `ConversationSummary` derivation to aggregate chat-text counts and
 * previews across every session in the resumption chain.
 */
export function listSessionIdsByConversationId(
  store: Store,
  conversationId: string,
): string[] {
  const items = store.list<Session>(COLLECTION, false);
  const matches: { id: string; creation_time: string }[] = [];
  for (const { id, entry } of items) {
    if (conversationIdOf(entry.data) !== conversationId) continue;
    matches.push({ id, creation_time: entry.data.creation_time ?? "" });
  }
  matches.sort((a, b) => {
    if (a.creation_time === b.creation_time) return a.id < b.id ? -1 : 1;
    return a.creation_time < b.creation_time ? -1 : 1;
  });
  return matches.map((m) => m.id);
}

/**
 * Spawn a fresh interactive session linked to `conversationId`. Mirrors the
 * real-backend resume-on-send behaviour: when a conversation has no live
 * session, the server materializes one and writes the user message to it.
 * Returns the new session id.
 */
export function createInteractiveSessionForConversation(
  store: Store,
  conversationId: string,
  now: string,
): string {
  const id = generateId("session");
  const task: Session = {
    mode: { type: "interactive", conversation_id: conversationId },
    agent_config: {},
    mount_spec: {
      working_dir: "repo",
      mounts: [
        {
          type: "bundle",
          target: "repo",
          bundle: { type: "none" },
        },
        { type: "documents", target: "documents" },
      ],
    },
    creator: DEV_USERNAME,
    status: "running" as Status,
    creation_time: now,
    start_time: now,
  };
  store.create<Session>(COLLECTION, id, task, SSE_PREFIX);
  return id;
}

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
    prompt: promptOf(task).slice(0, 100),
    spawned_from: task.spawned_from,
    conversation_id: conversationIdOf(task),
    creator: task.creator,
    status: task.status,
    error: task.error,
    deleted: task.deleted,
    creation_time: task.creation_time,
    start_time: task.start_time,
    end_time: task.end_time,
    usage: task.usage,
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
    const mountSpec =
      body.mount_spec.mounts.length === 0
        ? {
            working_dir: "repo",
            mounts: [
              { type: "bundle", target: "repo", bundle: { type: "none" } },
              { type: "documents", target: "documents" },
            ] as MountItem[],
          }
        : body.mount_spec;
    const task: Session = {
      mode: body.mode,
      agent_config: body.agent_config,
      mount_spec: mountSpec,
      spawned_from: body.spawned_from,
      resumed_from: body.resumed_from,
      creator: DEV_USERNAME,
      image: body.image,
      env_vars: body.env_vars,
      cpu_limit: body.cpu_limit,
      memory_limit: body.memory_limit,
      secrets: body.secrets,
      status: "pending" as Status,
      creation_time: now,
    };
    store.create<Session>(COLLECTION, id, task, SSE_PREFIX);
    const resp: CreateSessionResponse = { session_id: id, session: task };
    return c.json(resp, 201);
  });

  // GET /v1/sessions
  app.get("/v1/sessions", (c) => {
    const includeDeleted = c.req.query("include_deleted") === "true";
    const q = c.req.query("q");
    const spawnedFrom = c.req.query("spawned_from");
    const spawnedFromIds = c.req.query("spawned_from_ids");
    const status = c.req.query("status");
    const conversationId = c.req.query("conversation_id");
    const limitParam = c.req.query("limit");
    const countParam = c.req.query("count");

    const items = store.list<Session>(COLLECTION, includeDeleted);

    let filtered = items;
    if (q) {
      const lower = q.toLowerCase();
      filtered = filtered.filter(({ entry }) =>
        promptOf(entry.data).toLowerCase().includes(lower),
      );
    }
    if (spawnedFrom) {
      filtered = filtered.filter(({ entry }) => entry.data.spawned_from === spawnedFrom);
    }
    if (spawnedFromIds) {
      const ids = new Set(spawnedFromIds.split(",").map((s) => s.trim()));
      filtered = filtered.filter(({ entry }) => entry.data.spawned_from != null && ids.has(entry.data.spawned_from));
    }
    if (status) {
      const statuses = new Set(status.split(","));
      filtered = filtered.filter(({ entry }) => statuses.has(entry.data.status));
    }
    if (conversationId) {
      filtered = filtered.filter(
        ({ entry }) => conversationIdOf(entry.data) === conversationId,
      );
    }

    const totalCount = filtered.length;
    if (limitParam !== undefined && limitParam !== null) {
      const limit = Number(limitParam);
      if (Number.isFinite(limit) && limit >= 0) {
        filtered = filtered.slice(0, limit);
      }
    }

    const sessions: SessionSummaryRecord[] = filtered.map(({ id, entry }) =>
      toSummaryRecord(id, entry.version, entry.timestamp, entry.data),
    );
    const resp: ListSessionsResponse = {
      sessions,
      total_count: countParam === "true" ? BigInt(totalCount) : undefined,
    };
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
    // PR-F: WorkerContext.session is a straight read of the persisted
    // session; no per-fetch re-derivation of mount_spec from a separate
    // `context` field.
    const sessionWithEnv: Session = { ...task, env_vars: task.env_vars ?? {} };
    const resp: WorkerContext = {
      session: sessionWithEnv,
      resolved_env: task.env_vars ?? {},
      github_token: null,
      resumed_state: null,
    };
    return c.json(resp);
  });

  // GET /v1/sessions/:id/events — SessionEvent log for a single session.
  // Used by the Phase C step 11 chat read path; the frontend fans out over
  // every session linked to a conversation and concatenates the per-session
  // logs in creation-time order.
  app.get("/v1/sessions/:id/events", (c) => {
    const id = c.req.param("id");
    const entry = store.get<Session>(COLLECTION, id);
    if (!entry) {
      return c.json({ error: `session '${id}' not found` }, 404);
    }
    return c.json(getSessionEvents(id));
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
