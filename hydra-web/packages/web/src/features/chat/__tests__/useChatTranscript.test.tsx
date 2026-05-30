import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type {
  ListSessionsResponse,
  SessionEvent,
  SessionSummaryRecord,
} from "@hydra/api";

const mockListSessions = vi.fn();
const mockGetSessionEvents = vi.fn();

vi.mock("../../../api/client", () => ({
  apiClient: {
    listSessions: (...args: unknown[]) => mockListSessions(...args),
    getSessionEvents: (sid: string) => mockGetSessionEvents(sid),
  },
}));

const { useChatTranscript } = await import("../useChatTranscript");

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

describe("useChatTranscript", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("returns the SessionEvent merge when at least one linked session has rows", async () => {
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
      expect(result.current.events).toEqual([
        { type: "user_message", content: "new q", timestamp: "2026-01-01T00:00:30Z" },
        { type: "assistant_message", content: "new a", timestamp: "2026-01-01T00:00:45Z" },
      ]);
    });
  });

  it("returns an empty transcript when the conversation has no linked sessions", async () => {
    mockListSessions.mockResolvedValue(makeListSessionsResponse([]));

    const { result } = renderHook(() => useChatTranscript("c-1"), {
      wrapper: wrapper(),
    });

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });
    expect(result.current.events).toEqual([]);
    expect(mockGetSessionEvents).not.toHaveBeenCalled();
  });

  it("returns an empty transcript when every linked session has zero SessionEvent rows", async () => {
    mockListSessions.mockResolvedValue(
      makeListSessionsResponse([makeSessionSummary("t-1", "2026-01-01T00:00:00Z")]),
    );
    mockGetSessionEvents.mockResolvedValue([]);

    const { result } = renderHook(() => useChatTranscript("c-1"), {
      wrapper: wrapper(),
    });

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });
    expect(result.current.events).toEqual([]);
    expect(mockGetSessionEvents).toHaveBeenCalledWith("t-1");
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
            source: "transcript",
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

  it("keeps tool_use and unknown variants in the merged log (renderers drop them)", async () => {
    // The transcript hook is content-agnostic now: it returns the raw
    // SessionEvent stream and lets the renderer decide what to display.
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
      expect(result.current.events.length).toBeGreaterThan(0);
    });
    expect(result.current.events.map((e) => e.type)).toEqual([
      "user_message",
      "tool_use",
      "unknown",
      "assistant_message",
    ]);
  });
});
