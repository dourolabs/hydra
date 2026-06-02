import { describe, it, expect, beforeAll, afterAll } from "vitest";
import { startMockServer, type MockServerHandle, store } from "../index.js";
import { startSyntheticEvents } from "../synthetic-events.js";

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

describe("Synthetic SessionEvent generator", () => {
  it("appends session_event_created events for running sessions on the configured interval", async () => {
    // Pick a known running fixture session.
    const sessionsResp = await fetch(
      `${baseUrl}/v1/sessions?status=running&limit=1`,
      { headers: { Authorization: "Bearer dev-token-12345" } },
    );
    expect(sessionsResp.status).toBe(200);
    const { sessions } = (await sessionsResp.json()) as {
      sessions: Array<{ session_id: string }>;
    };
    expect(sessions.length).toBeGreaterThan(0);
    const targetSessionId = sessions[0].session_id;

    // Subscribe to /v1/events BEFORE starting the loop so we don't miss any
    // events between start and connect.
    const controller = new AbortController();
    const resp = await fetch(`${baseUrl}/v1/events`, {
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
      predicate: (evts: SseFrame[]) => boolean,
      timeoutMs: number,
    ): Promise<boolean> {
      const deadline = Date.now() + timeoutMs;
      while (!predicate(received) && Date.now() < deadline) {
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
      return predicate(received);
    }

    // Wait for the connected handshake before kicking off the generator.
    const connected = await readUntil(
      (evts) => evts.some((e) => e.event === "connected"),
      1000,
    );
    expect(connected).toBe(true);

    const handle = startSyntheticEvents(store, { intervalMs: 25 });
    try {
      const got = await readUntil((evts) => {
        return evts.some((e) => {
          if (e.event !== "session_event_created") return false;
          try {
            const payload = JSON.parse(e.data);
            return payload.entity_id === targetSessionId;
          } catch {
            return false;
          }
        });
      }, 1000);
      expect(got).toBe(true);

      // Inspect the matched event payload.
      const matched = received.find((e) => {
        if (e.event !== "session_event_created") return false;
        try {
          return JSON.parse(e.data).entity_id === targetSessionId;
        } catch {
          return false;
        }
      })!;
      const payload = JSON.parse(matched.data);
      expect(payload.entity_type).toBe("session_event");
      expect(payload.entity_id).toBe(targetSessionId);
      // Rotation includes tool_use and assistant_message; either is acceptable.
      expect(["tool_use", "assistant_message"]).toContain(payload.entity.type);
    } finally {
      handle.stop();
      controller.abort();
      reader.releaseLock();
    }
  });

  it("stop() is idempotent and silences further ticks", async () => {
    const handle = startSyntheticEvents(store, { intervalMs: 10 });
    handle.stop();
    handle.stop(); // must not throw
    const seqBefore = store.getCurrentSeq();
    await new Promise((r) => setTimeout(r, 60));
    expect(store.getCurrentSeq()).toBe(seqBefore);
  });
});
