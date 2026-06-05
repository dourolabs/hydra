import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor, act } from "@testing-library/react";
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

describe("useChatTranscript across mount/unmount/remount (production-like staleTime)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  // Repros the bug from [[i-dihnqqsc]]: the user opens a chat where the agent
  // is mid-flight (tail = user_message → "Thinking…"), navigates away, the
  // agent's reply lands while the page is unmounted, then the user navigates
  // back. Without forcing a refetch on mount, the React Query cache stays
  // "fresh" (production staleTime: 30s) for the brief navigation round-trip
  // and the indicator persists until a hard refresh.
  //
  // We model the off-screen reply by swapping the `getSessionEvents` mock
  // before the remount WITHOUT firing an SSE invalidation — i.e. the case
  // where the SSE event was missed (disconnect, race during navigation, or
  // the user came back inside the staleTime window before SSE-driven
  // invalidation could land).
  it("refetches session events on remount even when the cache is still fresh", async () => {
    const queryClient = new QueryClient({
      defaultOptions: {
        queries: {
          retry: false,
          // Mirror App.tsx production setting.
          staleTime: 30_000,
        },
      },
    });
    const wrapper = ({ children }: { children: React.ReactNode }) =>
      React.createElement(QueryClientProvider, { client: queryClient }, children);

    mockListSessions.mockResolvedValue(
      makeListSessionsResponse([makeSessionSummary("t-1", "2026-01-01T00:00:00Z")]),
    );
    // First fetch: tail is a user_message — the agent is mid-flight.
    mockGetSessionEvents.mockResolvedValueOnce([
      {
        type: "user_message",
        content: "do the thing",
        timestamp: "2026-01-01T00:00:30Z",
      },
    ] as SessionEvent[]);

    const first = renderHook(() => useChatTranscript("c-1"), { wrapper });
    await waitFor(() => {
      expect(first.result.current.events).toEqual([
        {
          type: "user_message",
          content: "do the thing",
          timestamp: "2026-01-01T00:00:30Z",
        },
      ]);
    });

    // User navigates away — unmount the chat page. The cache for
    // ["sessionEvents", "t-1"] sticks around at this point.
    first.unmount();

    // While the chat page is unmounted, the agent's reply lands on the
    // server. We deliberately do NOT call `invalidateQueries` here — that
    // models a missed SSE invalidation (the bug surface).
    mockGetSessionEvents.mockReset();
    mockGetSessionEvents.mockResolvedValue([
      {
        type: "user_message",
        content: "do the thing",
        timestamp: "2026-01-01T00:00:30Z",
      },
      {
        type: "assistant_message",
        content: "done",
        timestamp: "2026-01-01T00:00:45Z",
      },
    ] as SessionEvent[]);

    // User navigates back — remount the chat page.
    const second = renderHook(() => useChatTranscript("c-1"), { wrapper });

    // The page must reflect the agent's reply, not the stale cached
    // "user_message" tail. Wait until the second fetch lands.
    await waitFor(() => {
      expect(second.result.current.events.map((e) => e.type)).toEqual([
        "user_message",
        "assistant_message",
      ]);
    });

    // The mount-time refetch must hit the network for the per-session
    // events query — otherwise the indicator would not clear.
    expect(mockGetSessionEvents).toHaveBeenCalledWith("t-1");

    second.unmount();
    // Avoid a "act()" warning from any pending background refetch.
    await act(async () => {
      await Promise.resolve();
    });
  });
});
