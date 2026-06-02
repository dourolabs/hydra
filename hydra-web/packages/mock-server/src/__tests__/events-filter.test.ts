import { describe, it, expect, beforeAll, afterAll } from "vitest";
import type { SessionEvent } from "@hydra/api";
import { startMockServer, type MockServerHandle, store } from "../index.js";
import { appendSessionEvent } from "../routes/sessions.js";
import { eventCategory } from "../routes/events.js";

let server: MockServerHandle;
let baseUrl: string;

beforeAll(async () => {
  server = await startMockServer({ port: 0 });
  baseUrl = `http://localhost:${server.port}`;
});

afterAll(async () => {
  await server?.close();
});

interface SseFrame {
  event: string;
  data: string;
  id?: string;
}

function parseSSEBuffer(buf: string): { parsed: SseFrame[]; remaining: string } {
  const parsed: SseFrame[] = [];
  const blocks = buf.split("\n\n");
  const remaining = blocks.pop() ?? "";
  for (const block of blocks) {
    if (!block.trim()) continue;
    let event = "";
    let data = "";
    let id: string | undefined;
    for (const line of block.split("\n")) {
      if (line.startsWith("event:")) event = line.slice(6).trim();
      else if (line.startsWith("data:")) data = line.slice(5).trim();
      else if (line.startsWith("id:")) id = line.slice(3).trim();
    }
    if (event || data) parsed.push({ event, data, id });
  }
  return { parsed, remaining };
}

async function openStreamAndCollect(
  query: string,
  emit: () => void,
  predicate: (evts: SseFrame[]) => boolean,
  timeoutMs = 1000,
): Promise<{ frames: SseFrame[]; matched: boolean; controller: AbortController }> {
  const controller = new AbortController();
  const resp = await fetch(`${baseUrl}/v1/events?${query}`, {
    method: "GET",
    headers: { Authorization: "Bearer dev-token-12345" },
    signal: controller.signal,
  });
  expect(resp.status).toBe(200);
  const reader = resp.body!.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  const received: SseFrame[] = [];

  async function readUntil(
    p: (evts: SseFrame[]) => boolean,
    deadlineMs: number,
  ): Promise<boolean> {
    const deadline = Date.now() + deadlineMs;
    while (!p(received) && Date.now() < deadline) {
      const remaining = deadline - Date.now();
      if (remaining <= 0) break;
      const result = await Promise.race([
        reader.read(),
        new Promise<{ done: true; value: undefined }>((resolveTimeout) =>
          setTimeout(() => resolveTimeout({ done: true, value: undefined }), remaining),
        ),
      ]);
      if (result.done) break;
      buffer += decoder.decode(result.value, { stream: true });
      const { parsed, remaining: rem } = parseSSEBuffer(buffer);
      buffer = rem;
      received.push(...parsed);
    }
    return p(received);
  }

  // Wait for the connected handshake so the subscribe() callback is wired up
  // before we trigger the emission.
  const connected = await readUntil((evts) => evts.some((e) => e.event === "connected"), 1000);
  expect(connected).toBe(true);

  emit();

  const matched = await readUntil(predicate, timeoutMs);
  try {
    reader.releaseLock();
  } catch {
    // Already released; ignore.
  }
  controller.abort();
  return { frames: received, matched, controller };
}

function makeSessionEvent(): SessionEvent {
  return {
    type: "user_message",
    content: "filter-test message",
    timestamp: new Date().toISOString(),
  };
}

describe("eventCategory", () => {
  it("maps entity event types to their `types=` category", () => {
    expect(eventCategory("issue_created")).toBe("issues");
    expect(eventCategory("issue_updated")).toBe("issues");
    expect(eventCategory("issue_deleted")).toBe("issues");
    expect(eventCategory("patch_created")).toBe("patches");
    expect(eventCategory("session_created")).toBe("sessions");
    expect(eventCategory("session_event_created")).toBe("sessions");
    expect(eventCategory("session_state_updated")).toBe("sessions");
    expect(eventCategory("document_updated")).toBe("documents");
    expect(eventCategory("label_created")).toBe("labels");
    expect(eventCategory("conversation_created")).toBe("conversations");
    expect(eventCategory("conversation_event_created")).toBe("conversations");
  });

  it("returns null for non-entity event types", () => {
    expect(eventCategory("connected")).toBeNull();
    expect(eventCategory("resync")).toBeNull();
    expect(eventCategory("heartbeat")).toBeNull();
    expect(eventCategory("session_log")).toBeNull();
  });
});

describe("/v1/events typesFilter", () => {
  it("types=sessions delivers session_event_created", async () => {
    const sessionsResp = await fetch(`${baseUrl}/v1/sessions?status=running&limit=1`, {
      headers: { Authorization: "Bearer dev-token-12345" },
    });
    const { sessions } = (await sessionsResp.json()) as {
      sessions: Array<{ session_id: string }>;
    };
    const sessionId = sessions[0].session_id;

    const { matched } = await openStreamAndCollect(
      "types=sessions",
      () => appendSessionEvent(store, sessionId, makeSessionEvent()),
      (evts) =>
        evts.some((e) => {
          if (e.event !== "session_event_created") return false;
          try {
            return JSON.parse(e.data).entity_id === sessionId;
          } catch {
            return false;
          }
        }),
    );
    expect(matched).toBe(true);
  });

  it("types=issues rejects session_event_created", async () => {
    const sessionsResp = await fetch(`${baseUrl}/v1/sessions?status=running&limit=1`, {
      headers: { Authorization: "Bearer dev-token-12345" },
    });
    const { sessions } = (await sessionsResp.json()) as {
      sessions: Array<{ session_id: string }>;
    };
    const sessionId = sessions[0].session_id;

    const { frames, matched } = await openStreamAndCollect(
      "types=issues",
      () => appendSessionEvent(store, sessionId, makeSessionEvent()),
      // Predicate that resolves only when we see a session_event_created — we
      // expect it NEVER to fire under types=issues, so the readUntil call
      // should time out and matched should be false.
      (evts) => evts.some((e) => e.event === "session_event_created"),
      300,
    );
    expect(matched).toBe(false);
    // Sanity: the handshake still went through and the only events we saw
    // are infrastructure events that bypass the filter.
    for (const frame of frames) {
      expect(["connected", "heartbeat", "resync"]).toContain(frame.event);
    }
  });

  it("types=patches does not match session_event_created", async () => {
    const sessionsResp = await fetch(`${baseUrl}/v1/sessions?status=running&limit=1`, {
      headers: { Authorization: "Bearer dev-token-12345" },
    });
    const { sessions } = (await sessionsResp.json()) as {
      sessions: Array<{ session_id: string }>;
    };
    const sessionId = sessions[0].session_id;

    const { matched } = await openStreamAndCollect(
      "types=patches",
      () => appendSessionEvent(store, sessionId, makeSessionEvent()),
      (evts) => evts.some((e) => e.event === "session_event_created"),
      300,
    );
    expect(matched).toBe(false);
  });
});
