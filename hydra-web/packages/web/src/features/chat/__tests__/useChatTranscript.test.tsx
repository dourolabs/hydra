import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type {
  ConversationEvent,
  ListSessionsResponse,
  SessionEvent,
  SessionSummaryRecord,
} from "@hydra/api";

const mockListSessions = vi.fn();
const mockGetSessionEvents = vi.fn();
const mockGetConversationEvents = vi.fn();

vi.mock("../../../api/client", () => ({
  apiClient: {
    listSessions: (...args: unknown[]) => mockListSessions(...args),
    getSessionEvents: (sid: string) => mockGetSessionEvents(sid),
    getConversationEvents: (cid: string) => mockGetConversationEvents(cid),
  },
}));

const { useChatTranscript, sessionEventToConversationEvent } = await import(
  "../useChatTranscript"
);

function wrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return ({ children }: { children: React.ReactNode }) =>
    React.createElement(QueryClientProvider, { client: queryClient }, children);
}

function makeSessionSummary(
  sessionId: string,
  creationTime: string,
): SessionSummaryRecord {
  return {
    session_id: sessionId,
    version: 1n,
    timestamp: creationTime,
    session: {
      prompt: "",
      creator: "alice",
      status: "running",
      creation_time: creationTime,
    },
  } as SessionSummaryRecord;
}

function makeListSessionsResponse(
  records: SessionSummaryRecord[],
): ListSessionsResponse {
  return { sessions: records } as ListSessionsResponse;
}

describe("sessionEventToConversationEvent", () => {
  it("maps user_message verbatim", () => {
    const e: SessionEvent = {
      type: "user_message",
      content: "hi",
      timestamp: "t",
    };
    expect(sessionEventToConversationEvent(e)).toEqual({
      type: "user_message",
      content: "hi",
      timestamp: "t",
    });
  });

  it("maps assistant_message verbatim", () => {
    const e: SessionEvent = {
      type: "assistant_message",
      content: "hello",
      timestamp: "t",
    };
    expect(sessionEventToConversationEvent(e)).toEqual({
      type: "assistant_message",
      content: "hello",
      timestamp: "t",
    });
  });

  it("maps suspending with reason", () => {
    const e: SessionEvent = {
      type: "suspending",
      reason: "ctx limit",
      timestamp: "t",
    };
    expect(sessionEventToConversationEvent(e)).toEqual({
      type: "suspending",
      reason: "ctx limit",
      timestamp: "t",
    });
  });

  it("forwards SessionEvent.resumed.from_session_id into ConversationEvent.resumed.session_id", () => {
    const e: SessionEvent = {
      type: "resumed",
      from_session_id: "t-prev",
      timestamp: "t",
    };
    expect(sessionEventToConversationEvent(e)).toEqual({
      type: "resumed",
      session_id: "t-prev",
      timestamp: "t",
    });
  });

  it("drops tool_use (out of scope for the Phase C cut-over)", () => {
    const e: SessionEvent = {
      type: "tool_use",
      tool_name: "shell",
      payload: {} as never,
      timestamp: "t",
    };
    expect(sessionEventToConversationEvent(e)).toBeNull();
  });

  it("drops the forward-compat unknown variant", () => {
    expect(sessionEventToConversationEvent({ type: "unknown" })).toBeNull();
  });
});

describe("useChatTranscript", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("falls back to ConversationEvent when the conversation has no linked sessions", async () => {
    mockListSessions.mockResolvedValue(makeListSessionsResponse([]));
    const legacy: ConversationEvent[] = [
      { type: "user_message", content: "legacy q", timestamp: "2026-01-01T00:00:00Z" },
      { type: "assistant_message", content: "legacy a", timestamp: "2026-01-01T00:01:00Z" },
    ];
    mockGetConversationEvents.mockResolvedValue(legacy);

    const { result } = renderHook(() => useChatTranscript("c-1"), {
      wrapper: wrapper(),
    });

    await waitFor(() => {
      expect(result.current.source).toBe("conversation_events");
      expect(result.current.events).toEqual(legacy);
    });
    expect(mockGetSessionEvents).not.toHaveBeenCalled();
  });

  it("falls back to ConversationEvent when every linked session has zero SessionEvent rows", async () => {
    mockListSessions.mockResolvedValue(
      makeListSessionsResponse([makeSessionSummary("t-1", "2026-01-01T00:00:00Z")]),
    );
    mockGetSessionEvents.mockResolvedValue([]);
    const legacy: ConversationEvent[] = [
      { type: "user_message", content: "pre-rollout q", timestamp: "2026-01-01T00:00:00Z" },
    ];
    mockGetConversationEvents.mockResolvedValue(legacy);

    const { result } = renderHook(() => useChatTranscript("c-1"), {
      wrapper: wrapper(),
    });

    await waitFor(() => {
      expect(result.current.source).toBe("conversation_events");
      expect(result.current.events).toEqual(legacy);
    });
    expect(mockGetSessionEvents).toHaveBeenCalledWith("t-1");
  });

  it("uses the SessionEvent merge when at least one linked session has rows", async () => {
    mockListSessions.mockResolvedValue(
      makeListSessionsResponse([makeSessionSummary("t-1", "2026-01-01T00:00:00Z")]),
    );
    mockGetSessionEvents.mockResolvedValue([
      { type: "user_message", content: "new q", timestamp: "2026-01-01T00:00:30Z" },
      { type: "assistant_message", content: "new a", timestamp: "2026-01-01T00:00:45Z" },
    ] as SessionEvent[]);

    const { result } = renderHook(() => useChatTranscript("c-1"), {
      wrapper: wrapper(),
    });

    await waitFor(() => {
      expect(result.current.source).toBe("session_events");
      expect(result.current.events).toEqual([
        { type: "user_message", content: "new q", timestamp: "2026-01-01T00:00:30Z" },
        { type: "assistant_message", content: "new a", timestamp: "2026-01-01T00:00:45Z" },
      ]);
    });
    // The legacy fallback must not fire when the new path produced rows.
    expect(mockGetConversationEvents).not.toHaveBeenCalled();
  });

  it("concatenates per-session SessionEvent logs in creation-time order across a resumption chain", async () => {
    // List returns sessions in arbitrary order (the real server sorts by
    // timestamp DESC); the hook must re-sort by creation_time ASC.
    mockListSessions.mockResolvedValue(
      makeListSessionsResponse([
        makeSessionSummary("t-second", "2026-04-01T10:00:00.000Z"),
        makeSessionSummary("t-first", "2026-04-01T09:00:00.000Z"),
      ]),
    );
    mockGetSessionEvents.mockImplementation((sid: string) => {
      if (sid === "t-first") {
        return Promise.resolve([
          { type: "user_message", content: "q1", timestamp: "2026-04-01T09:01:00Z" },
          { type: "assistant_message", content: "a1", timestamp: "2026-04-01T09:02:00Z" },
          { type: "suspending", reason: "ctx", timestamp: "2026-04-01T09:30:00Z" },
        ] as SessionEvent[]);
      }
      if (sid === "t-second") {
        return Promise.resolve([
          {
            type: "resumed",
            from_session_id: "t-first",
            timestamp: "2026-04-01T10:00:30Z",
          },
          { type: "user_message", content: "q2", timestamp: "2026-04-01T10:05:00Z" },
          { type: "assistant_message", content: "a2", timestamp: "2026-04-01T10:10:00Z" },
        ] as SessionEvent[]);
      }
      return Promise.resolve([]);
    });

    const { result } = renderHook(() => useChatTranscript("c-1"), {
      wrapper: wrapper(),
    });

    await waitFor(() => {
      expect(result.current.source).toBe("session_events");
      expect(result.current.events).toHaveLength(6);
    });

    // First session's events come first, then second session's, with no
    // interleaving — the resumption protocol guarantees session N+1 events
    // are strictly later than session N's.
    expect(result.current.events.map((e) => e.type)).toEqual([
      "user_message",
      "assistant_message",
      "suspending",
      "resumed",
      "user_message",
      "assistant_message",
    ]);
    const contents = result.current.events.flatMap((e) =>
      "content" in e ? [e.content] : [],
    );
    expect(contents).toEqual(["q1", "a1", "q2", "a2"]);
  });

  it("drops tool_use and unknown variants from the merge", async () => {
    mockListSessions.mockResolvedValue(
      makeListSessionsResponse([makeSessionSummary("t-1", "2026-01-01T00:00:00Z")]),
    );
    mockGetSessionEvents.mockResolvedValue([
      { type: "user_message", content: "u", timestamp: "2026-01-01T00:00:30Z" },
      {
        type: "tool_use",
        tool_name: "shell",
        payload: {},
        timestamp: "2026-01-01T00:00:40Z",
      },
      { type: "unknown" },
      { type: "assistant_message", content: "a", timestamp: "2026-01-01T00:00:45Z" },
    ] as SessionEvent[]);

    const { result } = renderHook(() => useChatTranscript("c-1"), {
      wrapper: wrapper(),
    });

    await waitFor(() => {
      expect(result.current.source).toBe("session_events");
    });
    expect(result.current.events.map((e) => e.type)).toEqual([
      "user_message",
      "assistant_message",
    ]);
  });
});
