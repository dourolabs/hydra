import { Hono } from "hono";
import type { Store } from "../store.js";
import { generateId } from "../id.js";
import { DEV_USERNAME } from "../auth.js";
import {
  appendSessionEvent,
  conversationIdOf,
  createInteractiveSessionForConversation,
} from "./sessions.js";
import type {
  Conversation,
  ConversationEvent,
  ConversationStatus,
  ConversationSummary,
  CreateConversationRequest,
  SendMessageRequest,
  Session,
} from "@hydra/api";

const COLLECTION = "conversations";
const SSE_PREFIX = "conversation";

// Per-conversation event log. Kept module-local (parallel to routes/secrets.ts)
// so that loadSeedData can reset it via clearConversationEvents.
const conversationEvents = new Map<string, ConversationEvent[]>();

export function clearConversationEvents(): void {
  conversationEvents.clear();
}

export function setConversationEvents(
  conversationId: string,
  events: ConversationEvent[],
): void {
  conversationEvents.set(conversationId, [...events]);
}

function getEvents(conversationId: string): ConversationEvent[] {
  return conversationEvents.get(conversationId) ?? [];
}

function appendEvent(conversationId: string, event: ConversationEvent): void {
  const existing = conversationEvents.get(conversationId);
  if (existing) {
    existing.push(event);
  } else {
    conversationEvents.set(conversationId, [event]);
  }
}

function eventPreview(event: ConversationEvent): string {
  switch (event.type) {
    case "suspending":
      return `Suspending: ${event.reason}`;
    case "resumed":
      return "Resumed";
    case "closed":
      return "Closed";
  }
}

function toSummary(
  id: string,
  conversation: Conversation,
): ConversationSummary {
  const events = getEvents(id);
  const last = events.length > 0 ? events[events.length - 1] : null;
  return {
    conversation_id: id,
    title: conversation.title,
    agent_name: conversation.agent_name,
    status: conversation.status,
    event_count: events.length,
    last_event_preview: last ? eventPreview(last).slice(0, 200) : null,
    creator: conversation.creator,
    created_at: conversation.created_at,
    updated_at: conversation.updated_at,
  };
}

// Locate the most-recently-created interactive session linked to this
// conversation. Mirrors the real backend's resolve-session-for-conversation
// path used by send_message.
function latestSessionForConversation(
  store: Store,
  conversationId: string,
): string | null {
  const items = store.list<Session>("sessions", false);
  let chosen: { id: string; creation_time: string } | null = null;
  for (const { id, entry } of items) {
    if (conversationIdOf(entry.data) !== conversationId) continue;
    const ct = entry.data.creation_time ?? "";
    if (!chosen || ct > chosen.creation_time) {
      chosen = { id, creation_time: ct };
    }
  }
  return chosen?.id ?? null;
}

export function createConversationRoutes(store: Store): Hono {
  const app = new Hono();

  // GET /v1/conversations
  app.get("/v1/conversations", (c) => {
    const includeDeleted = c.req.query("include_deleted") === "true";
    const q = c.req.query("q");
    const status = c.req.query("status") as ConversationStatus | undefined;
    const creator = c.req.query("creator");
    const limitParam = c.req.query("limit");

    const items = store.list<Conversation>(COLLECTION, includeDeleted);

    let filtered = items;
    if (q) {
      const lower = q.toLowerCase();
      filtered = filtered.filter(({ id, entry }) => {
        const title = entry.data.title ?? "";
        const agent = entry.data.agent_name ?? "";
        return (
          title.toLowerCase().includes(lower) ||
          agent.toLowerCase().includes(lower) ||
          id.toLowerCase().includes(lower)
        );
      });
    }
    if (status) {
      filtered = filtered.filter(({ entry }) => entry.data.status === status);
    }
    if (creator) {
      filtered = filtered.filter(({ entry }) => entry.data.creator === creator);
    }

    if (limitParam !== undefined && limitParam !== null) {
      const limit = Number(limitParam);
      if (Number.isFinite(limit) && limit >= 0) {
        filtered = filtered.slice(0, limit);
      }
    }

    const summaries: ConversationSummary[] = filtered.map(({ id, entry }) =>
      toSummary(id, entry.data),
    );
    return c.json(summaries);
  });

  // GET /v1/conversations/:id
  app.get("/v1/conversations/:id", (c) => {
    const id = c.req.param("id");
    const entry = store.get<Conversation>(COLLECTION, id);
    if (!entry) {
      return c.json({ error: `conversation '${id}' not found` }, 404);
    }
    return c.json(entry.data);
  });

  // GET /v1/conversations/:id/events
  app.get("/v1/conversations/:id/events", (c) => {
    const id = c.req.param("id");
    const entry = store.get<Conversation>(COLLECTION, id);
    if (!entry) {
      return c.json({ error: `conversation '${id}' not found` }, 404);
    }
    return c.json(getEvents(id));
  });

  // POST /v1/conversations
  app.post("/v1/conversations", async (c) => {
    const body = await c.req.json<CreateConversationRequest>().catch(() => ({} as CreateConversationRequest));
    const id = generateId("conversation");
    const now = new Date().toISOString();
    const conversation: Conversation = {
      conversation_id: id,
      title: null,
      agent_name: body.agent_name ?? null,
      status: "active",
      creator: DEV_USERNAME,
      session_settings: body.session_settings ?? undefined,
      created_at: now,
      updated_at: now,
    };
    store.create<Conversation>(COLLECTION, id, conversation, SSE_PREFIX);
    if (body.message) {
      const sessionId = createInteractiveSessionForConversation(store, id, now);
      appendSessionEvent(sessionId, {
        type: "user_message",
        content: body.message,
        timestamp: now,
      });
    }
    return c.json(conversation, 201);
  });

  // POST /v1/conversations/:id/messages
  app.post("/v1/conversations/:id/messages", async (c) => {
    const id = c.req.param("id");
    const entry = store.get<Conversation>(COLLECTION, id);
    if (!entry) {
      return c.json({ error: `conversation '${id}' not found` }, 404);
    }
    const body = await c.req.json<SendMessageRequest>();
    const now = new Date().toISOString();
    const sessionId =
      latestSessionForConversation(store, id) ??
      createInteractiveSessionForConversation(store, id, now);
    appendSessionEvent(sessionId, {
      type: "user_message",
      content: body.content,
      timestamp: now,
    });
    const updated: Conversation = { ...entry.data, updated_at: now };
    store.update<Conversation>(COLLECTION, id, updated, SSE_PREFIX);
    return c.json(null);
  });

  // POST /v1/conversations/:id/close
  app.post("/v1/conversations/:id/close", (c) => {
    const id = c.req.param("id");
    const entry = store.get<Conversation>(COLLECTION, id);
    if (!entry) {
      return c.json({ error: `conversation '${id}' not found` }, 404);
    }
    const now = new Date().toISOString();
    appendEvent(id, { type: "closed", timestamp: now });
    const updated: Conversation = { ...entry.data, status: "closed", updated_at: now };
    store.update<Conversation>(COLLECTION, id, updated, SSE_PREFIX);
    return c.json(null);
  });

  // POST /v1/conversations/:id/resume
  app.post("/v1/conversations/:id/resume", (c) => {
    const id = c.req.param("id");
    const entry = store.get<Conversation>(COLLECTION, id);
    if (!entry) {
      return c.json({ error: `conversation '${id}' not found` }, 404);
    }
    const now = new Date().toISOString();
    const updated: Conversation = { ...entry.data, status: "active", updated_at: now };
    store.update<Conversation>(COLLECTION, id, updated, SSE_PREFIX);
    return c.json(updated);
  });

  return app;
}
