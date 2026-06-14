import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";

// --- EventSource mock ---
// A minimal stub that records typed listeners so tests can dispatch synthetic
// SSE events into the hook.

interface MockEvent {
  data: string;
  lastEventId?: string;
}

class MockEventSource {
  static OPEN = 1;
  static CLOSED = 2;
  static instances: MockEventSource[] = [];

  url: string;
  readyState = MockEventSource.OPEN;
  onopen: ((this: MockEventSource, ev: Event) => unknown) | null = null;
  onerror: ((this: MockEventSource, ev: Event) => unknown) | null = null;
  private listeners = new Map<string, Array<(e: MockEvent) => void>>();

  constructor(url: string) {
    this.url = url;
    MockEventSource.instances.push(this);
  }

  // Test helper: fire the onopen handler. Wrap calls in `act()` so any state
  // updates triggered by the handler are flushed inside the test.
  fireOpen() {
    this.onopen?.call(this, new Event("open"));
  }

  addEventListener(type: string, listener: (e: MockEvent) => void) {
    const arr = this.listeners.get(type) ?? [];
    arr.push(listener);
    this.listeners.set(type, arr);
  }

  removeEventListener(type: string, listener: (e: MockEvent) => void) {
    const arr = this.listeners.get(type);
    if (!arr) return;
    this.listeners.set(
      type,
      arr.filter((l) => l !== listener),
    );
  }

  close() {
    this.readyState = MockEventSource.CLOSED;
  }

  // Test helper: dispatch a synthetic event to all listeners of `type`.
  dispatch(type: string, payload: unknown, lastEventId = "1") {
    const listeners = this.listeners.get(type) ?? [];
    const event: MockEvent = {
      data: JSON.stringify(payload),
      lastEventId,
    };
    for (const listener of listeners) {
      listener(event);
    }
  }
}

// Stub on the global before importing the hook.
beforeEach(() => {
  MockEventSource.instances = [];
  (globalThis as unknown as { EventSource: typeof MockEventSource }).EventSource = MockEventSource;
});

afterEach(() => {
  for (const es of MockEventSource.instances) {
    es.close();
  }
  MockEventSource.instances = [];
});

const { useSSE } = await import("../useSSE");

function makeWrapper(client: QueryClient) {
  return function Wrapper({ children }: { children: React.ReactNode }) {
    return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
  };
}

function makeIssueRecord(issueId: string) {
  return {
    issue_id: issueId,
    version: 1,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    issue: {
      type: "task",
      title: `Issue ${issueId}`,
      description: "desc",
      creator: "alice",
      status: "open",
      progress: "",
      dependencies: [],
      patches: [],
      labels: [],
    },
  };
}

function makeSessionRecord(sessionId: string, issueId?: string) {
  return {
    session_id: sessionId,
    version: 1,
    timestamp: "2026-01-01T00:00:00Z",
    session: {
      prompt: "do work",
      spawned_from: issueId,
      creator: "alice",
      status: "running",
    },
  };
}

function makePatchRecord(patchId: string) {
  return {
    patch_id: patchId,
    version: 1,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    patch: {
      title: "Patch",
      status: "open",
      is_automatic_backup: false,
      creator: "alice",
      review_summary: { count: 0, approved: false },
      service_repo_name: "test-repo",
    },
  };
}

function makeDocumentRecord(documentId: string) {
  return {
    document_id: documentId,
    version: 1,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    document: {
      title: "Doc",
      path: `docs/${documentId}.md`,
      archived: false,
      labels: [],
    },
  };
}

interface InvalidateSpy {
  mock: { calls: unknown[][] };
  mockClear: () => void;
}

/**
 * Returns true if any invalidateQueries call would, under React Query
 * semantics, invalidate a query with the given key. React Query invalidates
 * queries whose key starts with the invalidation queryKey, so a stored
 * invalidation call with key K matches any search key that has K as a prefix.
 */
function wasInvalidated(spy: InvalidateSpy, key: readonly unknown[]): boolean {
  return spy.mock.calls.some((call) => {
    const arg = call[0] as { queryKey?: readonly unknown[] } | undefined;
    if (!arg?.queryKey) return false;
    if (arg.queryKey.length > key.length) return false;
    return arg.queryKey.every((k, i) => k === key[i]);
  });
}

describe("useSSE chatRelated cache invalidation", () => {
  let queryClient: QueryClient;
  let invalidateSpy: InvalidateSpy;

  beforeEach(() => {
    queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    invalidateSpy = vi.spyOn(queryClient, "invalidateQueries") as unknown as InvalidateSpy;
  });

  it("invalidates chatRelated keys on issue_updated", () => {
    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });

    expect(MockEventSource.instances.length).toBe(1);
    const es = MockEventSource.instances[0];

    act(() => {
      es.dispatch("issue_updated", {
        entity_type: "issue",
        entity_id: "i-1",
        version: 1,
        timestamp: "2026-01-01T00:00:00Z",
        entity: makeIssueRecord("i-1"),
      });
    });

    expect(wasInvalidated(invalidateSpy, ["chatRelated"])).toBe(true);
  });

  it("invalidates chatRelated keys on issue_deleted", () => {
    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    const es = MockEventSource.instances[0];

    act(() => {
      es.dispatch("issue_deleted", {
        entity_type: "issue",
        entity_id: "i-1",
        version: 2,
        timestamp: "2026-01-01T00:00:00Z",
        entity: makeIssueRecord("i-1"),
      });
    });

    expect(wasInvalidated(invalidateSpy, ["chatRelated"])).toBe(true);
  });

  it("does not invalidate chatRelated on session_updated", () => {
    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    const es = MockEventSource.instances[0];

    act(() => {
      es.dispatch("session_updated", {
        entity_type: "session",
        entity_id: "s-1",
        version: 1,
        timestamp: "2026-01-01T00:00:00Z",
        entity: makeSessionRecord("s-1", "i-1"),
      });
    });

    expect(wasInvalidated(invalidateSpy, ["chatRelated"])).toBe(false);
  });

  it("invalidates chatRelated on patch_created", () => {
    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    const es = MockEventSource.instances[0];

    act(() => {
      es.dispatch("patch_created", {
        entity_type: "patch",
        entity_id: "p-1",
        version: 1,
        timestamp: "2026-01-01T00:00:00Z",
        entity: makePatchRecord("p-1"),
      });
    });

    expect(wasInvalidated(invalidateSpy, ["chatRelated"])).toBe(true);
  });

  it("invalidates chatRelated on document_created", () => {
    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    const es = MockEventSource.instances[0];

    act(() => {
      es.dispatch("document_created", {
        entity_type: "document",
        entity_id: "d-1",
        version: 1,
        timestamp: "2026-01-01T00:00:00Z",
        entity: makeDocumentRecord("d-1"),
      });
    });

    expect(wasInvalidated(invalidateSpy, ["chatRelated"])).toBe(true);
  });

  it("invalidates current Related tab sub-keys on issue_created", () => {
    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    const es = MockEventSource.instances[0];

    act(() => {
      es.dispatch("issue_created", {
        entity_type: "issue",
        entity_id: "i-1",
        version: 1,
        timestamp: "2026-01-01T00:00:00Z",
        entity: makeIssueRecord("i-1"),
      });
    });

    // The broad ["chatRelated"] invalidation must cover every sub-key the
    // current useChatReferencedArtifacts hook reads from. wasInvalidated does
    // prefix-matching, so these all match the broad invalidation.
    expect(wasInvalidated(invalidateSpy, ["chatRelated", "refers-to"])).toBe(true);
    expect(wasInvalidated(invalidateSpy, ["chatRelated", "referencedIssues"])).toBe(true);
    expect(wasInvalidated(invalidateSpy, ["chatRelated", "referencedPatches"])).toBe(true);
    expect(wasInvalidated(invalidateSpy, ["chatRelated", "referencedDocuments"])).toBe(true);
  });

  it("prepends a newly-created session into the ['sessions', 'active'] cache when it matches", () => {
    // The sidebar's useActiveSessions hook uses key
    // ["sessions", "active", creator, limit]. We replaced the prior broad
    // invalidate with an in-place upsert so the sidebar live-updates without
    // a refetch.
    queryClient.setQueryData<unknown[]>(["sessions", "active", "alice", 6], []);

    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    const es = MockEventSource.instances[0];

    act(() => {
      es.dispatch("session_created", {
        entity_type: "session",
        entity_id: "s-1",
        version: 1,
        timestamp: "2026-01-01T00:00:00Z",
        entity: makeSessionRecord("s-1", "i-1"),
      });
    });

    const cached = queryClient.getQueryData<Array<{ session_id: string }>>([
      "sessions",
      "active",
      "alice",
      6,
    ]);
    expect(cached?.map((s) => s.session_id)).toEqual(["s-1"]);
    // The pre-existing activeCount invalidate must still fire alongside it
    // (count can't be patched from a single-record payload).
    expect(wasInvalidated(invalidateSpy, ["sessions", "activeCount"])).toBe(true);
  });

  it("removes a session from ['sessions', 'active'] when its status flips out of active", () => {
    // Seed with a running session so the upsert has something to find before
    // the update flips its status to "complete".
    queryClient.setQueryData(["sessions", "active", "alice", 6], [makeSessionRecord("s-1", "i-1")]);

    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    const es = MockEventSource.instances[0];

    act(() => {
      es.dispatch("session_updated", {
        entity_type: "session",
        entity_id: "s-1",
        version: 2,
        timestamp: "2026-01-01T00:00:00Z",
        entity: {
          ...makeSessionRecord("s-1", "i-1"),
          version: 2,
          session: { ...makeSessionRecord("s-1", "i-1").session, status: "complete" },
        },
      });
    });

    const cached = queryClient.getQueryData<Array<{ session_id: string }>>([
      "sessions",
      "active",
      "alice",
      6,
    ]);
    expect(cached?.map((s) => s.session_id)).toEqual([]);
  });

  it("invalidates ['proxyTargets', session_id] on session_updated so the ProxyTab live-updates when a worker advertises a port mid-conversation", () => {
    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    const es = MockEventSource.instances[0];

    act(() => {
      es.dispatch("session_updated", {
        entity_type: "session",
        entity_id: "s-proxy",
        version: 2,
        timestamp: "2026-01-01T00:00:00Z",
        entity: makeSessionRecord("s-proxy", "i-1"),
      });
    });

    // The Proxy tab keys its read on ["proxyTargets", activeSessionId]. Without
    // this invalidation, advertising a port mid-session leaves the cache stale
    // until the user navigates away and back (refetchOnMount: "always").
    expect(wasInvalidated(invalidateSpy, ["proxyTargets", "s-proxy"])).toBe(true);
  });

  it("routes session_updated through the SessionSummaryRecord arm even when the mock-server uses the plural entity_type 'sessions'", () => {
    // The mock-server emits entity_type = collection name (`sessions`, plural);
    // the real server emits `session` (singular). The dispatch must accept
    // both — otherwise mock-driven integration tests silently lose every session
    // mutation event.
    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    const es = MockEventSource.instances[0];

    act(() => {
      es.dispatch("session_updated", {
        entity_type: "sessions",
        entity_id: "s-plural",
        version: 2,
        timestamp: "2026-01-01T00:00:00Z",
        entity: makeSessionRecord("s-plural", "i-1"),
      });
    });

    expect(wasInvalidated(invalidateSpy, ["session", "s-plural"])).toBe(true);
    expect(wasInvalidated(invalidateSpy, ["proxyTargets", "s-plural"])).toBe(true);
  });

  it("patches ['sessions', 'active'] on session_updated without spawned_from", () => {
    // The no-spawned-from branch must still flow the new record into the
    // sidebar's active-sessions cache — we replaced the broad invalidate
    // with an in-place upsert, so the running session must update in place.
    queryClient.setQueryData(["sessions", "active", "alice", 6], [makeSessionRecord("s-2")]);

    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    const es = MockEventSource.instances[0];

    const updated = {
      ...makeSessionRecord("s-2"),
      version: 2,
      session: {
        ...makeSessionRecord("s-2").session,
        prompt: "different prompt",
      },
    };
    act(() => {
      es.dispatch("session_updated", {
        entity_type: "session",
        entity_id: "s-2",
        version: 2,
        timestamp: "2026-01-01T00:00:00Z",
        entity: updated,
      });
    });

    const cached = queryClient.getQueryData<Array<{ session: { prompt: string } }>>([
      "sessions",
      "active",
      "alice",
      6,
    ]);
    expect(cached?.[0]?.session.prompt).toBe("different prompt");
  });

  it("upserts a conversation into the ['conversations', 'batch', ids] cache on conversation_created", () => {
    // The board view's useActiveConversationsByIssue caches a wrapped
    // ListConversationsResponse (`{ conversations, next_cursor }`). The prior
    // implementation typed the cache as a flat array and silently failed to
    // update it — leaving the user with no "Go to conversation" affordance
    // until a manual refresh.
    queryClient.setQueryData(["conversations", "batch", "i-a,i-b"], {
      conversations: [],
      next_cursor: null,
    });

    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    const es = MockEventSource.instances[0];

    const newConversation = {
      conversation_id: "c-1",
      title: null,
      agent_name: null,
      status: "active",
      event_count: 0,
      last_event_preview: null,
      creator: "alice",
      spawned_from: "i-a",
      created_at: "2026-06-09T00:00:00Z",
      updated_at: "2026-06-09T00:00:00Z",
    };

    act(() => {
      es.dispatch("conversation_created", {
        entity_type: "conversation",
        entity_id: "c-1",
        version: 1,
        timestamp: "2026-06-09T00:00:00Z",
        entity: newConversation,
      });
    });

    const cached = queryClient.getQueryData<{
      conversations: Array<{ conversation_id: string }>;
    }>(["conversations", "batch", "i-a,i-b"]);
    expect(cached?.conversations.map((c) => c.conversation_id)).toEqual(["c-1"]);
  });

  it("invalidates chatRelated root on resync (after first event)", async () => {
    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    const es = MockEventSource.instances[0];

    // Resync invalidation only fires after at least one prior event has set
    // lastEventIdRef (the SSE hook's reconnect-recovery path also gates on
    // lastEventIdRef). Here we test the explicit `resync` event listener,
    // which always calls debouncedInvalidate.
    invalidateSpy.mockClear();

    act(() => {
      es.dispatch("resync", {}, "");
    });

    // debouncedInvalidate uses setTimeout(100). Advance with real timers.
    await new Promise((resolve) => setTimeout(resolve, 150));

    expect(wasInvalidated(invalidateSpy, ["chatRelated"])).toBe(true);
  });
});

interface SetQueriesDataSpy {
  mock: { calls: unknown[][] };
}

describe("useSSE SessionEvent / SessionState live-tail wiring", () => {
  let queryClient: QueryClient;
  let invalidateSpy: InvalidateSpy;
  let setQueriesDataSpy: SetQueriesDataSpy;

  beforeEach(() => {
    queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    invalidateSpy = vi.spyOn(queryClient, "invalidateQueries") as unknown as InvalidateSpy;
    setQueriesDataSpy = vi.spyOn(queryClient, "setQueriesData") as unknown as SetQueriesDataSpy;
  });

  it("appends the carried SessionEvent into ['sessionEvents', sid] on session_event_created", () => {
    // Seed the per-session events cache so the append finds an existing array.
    const existing = {
      type: "user_message",
      content: "first",
      timestamp: "2026-05-23T16:00:00Z",
    };
    queryClient.setQueryData(["sessionEvents", "t-abc"], [existing]);

    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    const es = MockEventSource.instances[0];

    const appended = {
      type: "user_message",
      content: "second",
      timestamp: "2026-05-23T17:00:00Z",
    };

    invalidateSpy.mockClear();

    act(() => {
      es.dispatch("session_event_created", {
        entity_type: "session_event",
        entity_id: "t-abc",
        version: 4,
        timestamp: "2026-05-23T17:00:00Z",
        entity: appended,
      });
    });

    const cached = queryClient.getQueryData<unknown[]>(["sessionEvents", "t-abc"]);
    expect(cached).toEqual([existing, appended]);

    // The append-into-cache path must NOT invalidate either the per-session
    // events query (avoid an unnecessary refetch when the carried payload is
    // already authoritative) or the conversation→sessions index (only changes
    // on session_created / session_updated).
    expect(wasInvalidated(invalidateSpy, ["sessionEvents", "t-abc"])).toBe(false);
    expect(wasInvalidated(invalidateSpy, ["sessionsByConversation"])).toBe(false);
  });

  it("no-ops the append when ['sessionEvents', sid] is undefined (first-load race)", () => {
    // Same race semantics as the existing sessionLogRegistry: if the cache
    // hasn't been populated yet, the in-flight initial fetch will pick up the
    // new event naturally — so we must leave the slot undefined, not write [evt].
    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    const es = MockEventSource.instances[0];

    act(() => {
      es.dispatch("session_event_created", {
        entity_type: "session_event",
        entity_id: "t-unseeded",
        version: 1,
        timestamp: "2026-05-23T17:00:00Z",
        entity: {
          type: "user_message",
          content: "hi",
          timestamp: "2026-05-23T17:00:00Z",
        },
      });
    });

    expect(queryClient.getQueryData(["sessionEvents", "t-unseeded"])).toBeUndefined();
  });

  it("does not poison ['sessions', 'batch'] when a session_event_created arrives", () => {
    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    const es = MockEventSource.instances[0];

    act(() => {
      es.dispatch("session_event_created", {
        entity_type: "session_event",
        entity_id: "t-abc",
        version: 1,
        timestamp: "2026-05-23T17:00:00Z",
        entity: {
          type: "user_message",
          content: "hi",
          timestamp: "2026-05-23T17:00:00Z",
        },
      });
    });

    // Regression guard: the prior implementation hit the SessionSummaryRecord
    // arm via `eventType.startsWith("session_")` and called
    // upsertBatchSession with a SessionEvent payload, poisoning the cache.
    // The new arm must NEVER write into ["sessions", "batch"] for these
    // events. setQueriesData calls with that key indicate a regression.
    const wroteToBatchSessions = setQueriesDataSpy.mock.calls.some((call) => {
      const filter = call[0] as { queryKey?: readonly unknown[] } | undefined;
      const key = filter?.queryKey;
      return Array.isArray(key) && key[0] === "sessions" && key[1] === "batch";
    });
    expect(wroteToBatchSessions).toBe(false);
  });

  it("does not invalidate ['session', sid] (per-session detail) on session_event_created", () => {
    // Sanity: the per-session-detail cache (keyed differently) shouldn't be
    // pulled in by the session_event arm. Keeps the invalidation set tight.
    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    const es = MockEventSource.instances[0];

    act(() => {
      es.dispatch("session_event_created", {
        entity_type: "session_event",
        entity_id: "t-abc",
        version: 1,
        timestamp: "2026-05-23T17:00:00Z",
        entity: {
          type: "user_message",
          content: "hi",
          timestamp: "2026-05-23T17:00:00Z",
        },
      });
    });

    expect(wasInvalidated(invalidateSpy, ["session", "t-abc"])).toBe(false);
  });

  it("invalidates ['sessionState', sid] on session_state_updated (entity is null)", () => {
    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    const es = MockEventSource.instances[0];

    act(() => {
      // SessionState SSE notifications deliberately carry no entity payload —
      // the early null-entity guard must let them through.
      es.dispatch("session_state_updated", {
        entity_type: "session_state",
        entity_id: "t-xyz",
        version: 0,
        timestamp: "2026-05-23T17:00:00Z",
      });
    });

    expect(wasInvalidated(invalidateSpy, ["sessionState", "t-xyz"])).toBe(true);
    // Must not fall through to the session-record arm.
    expect(wasInvalidated(invalidateSpy, ["session", "t-xyz"])).toBe(false);
  });

  it("still routes session_updated to the SessionSummaryRecord arm", () => {
    // Sibling regression guard: gating the existing arm on entity_type ===
    // "session" must not break ordinary session_updated handling.
    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    const es = MockEventSource.instances[0];

    act(() => {
      es.dispatch("session_updated", {
        entity_type: "session",
        entity_id: "s-1",
        version: 2,
        timestamp: "2026-05-23T17:00:00Z",
        entity: makeSessionRecord("s-1", "i-1"),
      });
    });

    // Per-session detail still invalidated.
    expect(wasInvalidated(invalidateSpy, ["session", "s-1"])).toBe(true);
    // And the new session_event-only keys are NOT touched by this event.
    expect(wasInvalidated(invalidateSpy, ["sessionEvents", "s-1"])).toBe(false);
  });
});

describe("useSSE reconnect: jittered backoff + force-close on visible/online", () => {
  let queryClient: QueryClient;

  beforeEach(() => {
    queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  // Loose alias so the helper accepts whatever `vi.spyOn(globalThis, "setTimeout")`
  // produces across vitest versions without us having to nail the generic
  // arguments exactly — we only need `.mock.calls`.
  type SetTimeoutLikeSpy = { mock: { calls: unknown[][] } };

  /**
   * Trigger the hook's onerror handler on a given mock instance and return
   * the delay the handler scheduled its reconnect setTimeout with. Filters
   * setTimeout calls to ones whose delay falls inside the backoff window
   * [BASE_BACKOFF_MS/2, MAX_BACKOFF_MS] so we don't mis-pick the 100ms
   * debouncedInvalidate timer or other unrelated schedules.
   */
  function fireErrorAndCaptureDelay(
    setTimeoutSpy: SetTimeoutLikeSpy,
    es: MockEventSource,
  ): { delay: number; reconnect: () => void } {
    const callsBefore = setTimeoutSpy.mock.calls.length;
    act(() => {
      es.onerror?.call(es, new Event("error"));
    });
    const scheduled = setTimeoutSpy.mock.calls.slice(callsBefore).find((c) => {
      const ms = c[1];
      return typeof ms === "number" && ms >= 500 && ms <= 15_000;
    });
    if (!scheduled) {
      throw new Error("Expected reconnect setTimeout to be scheduled after onerror");
    }
    return {
      delay: scheduled[1] as number,
      reconnect: scheduled[0] as () => void,
    };
  }

  it("half-jitters the reconnect delay (proves jitter is wired)", () => {
    // Half-jitter formula: delay = ceiling * (0.5 + Math.random() * 0.5).
    // Pin Math.random() to 0 → expect delay = BASE_BACKOFF_MS * 0.5 = 500.
    // Without jitter the raw exponential delay would be BASE_BACKOFF_MS = 1000.
    vi.spyOn(Math, "random").mockReturnValue(0);
    const setTimeoutSpy = vi.spyOn(globalThis, "setTimeout");

    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    expect(MockEventSource.instances.length).toBe(1);

    const { delay } = fireErrorAndCaptureDelay(setTimeoutSpy, MockEventSource.instances[0]);
    expect(delay).toBe(500);
  });

  it("two onerrors in quick succession produce two different delay values", () => {
    // With pinned-but-distinct Math.random() returns, half-jitter must yield
    // measurably different delays. This is the issue's "proves jitter is
    // wired" test; even if retries were not incrementing, jitter alone would
    // make the two delays differ.
    const randomSpy = vi.spyOn(Math, "random");
    randomSpy.mockReturnValueOnce(0).mockReturnValueOnce(0.99);
    const setTimeoutSpy = vi.spyOn(globalThis, "setTimeout");

    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    const first = fireErrorAndCaptureDelay(setTimeoutSpy, MockEventSource.instances[0]);

    // Drive the scheduled reconnect inline (instead of waiting wall-clock)
    // so the second instance comes up and we can fire a second onerror.
    act(() => {
      first.reconnect();
    });
    expect(MockEventSource.instances.length).toBe(2);

    const second = fireErrorAndCaptureDelay(setTimeoutSpy, MockEventSource.instances[1]);

    expect(first.delay).not.toBe(second.delay);
  });

  it("caps backoff at the lowered ceiling (15s, not 30s)", () => {
    // Force the ceiling-hit case: many retries with Math.random()=1 picks the
    // top of the jitter range. The delay must be <= 15s, the new ceiling.
    vi.spyOn(Math, "random").mockReturnValue(0.999_999);
    const setTimeoutSpy = vi.spyOn(globalThis, "setTimeout");

    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });

    let current: { delay: number; reconnect: () => void } | null = null;
    // Simulate 8 consecutive failures so the exponential ceiling clearly
    // exceeds 15s (2^8 * 1000 = 256_000ms) — every subsequent delay must be
    // clamped to MAX_BACKOFF_MS.
    for (let i = 0; i < 8; i++) {
      const target =
        current === null
          ? MockEventSource.instances[0]
          : (() => {
              act(() => {
                current!.reconnect();
              });
              return MockEventSource.instances[MockEventSource.instances.length - 1];
            })();
      current = fireErrorAndCaptureDelay(setTimeoutSpy, target);
    }

    expect(current).not.toBeNull();
    expect(current!.delay).toBeLessThanOrEqual(15_000);
    // And the floor of half-jitter at the ceiling is 7.5s, so the final delay
    // must be at least that (proves we hit the ceiling, not some lower rung).
    expect(current!.delay).toBeGreaterThanOrEqual(7_500);
  });

  it("force-closes the EventSource on visibility-change-to-visible (bypasses readyState guard)", () => {
    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    expect(MockEventSource.instances.length).toBe(1);

    const original = MockEventSource.instances[0];
    // Precondition: the bug scenario — readyState reads OPEN even though
    // the underlying socket is half-open after suspend.
    expect(original.readyState).toBe(MockEventSource.OPEN);

    act(() => {
      document.dispatchEvent(new Event("visibilitychange"));
    });

    // The handler must close() the prior instance and construct a new one.
    // (If the readyState guard inside connect() weren't being bypassed, no
    // new EventSource would be constructed.)
    expect(original.readyState).toBe(MockEventSource.CLOSED);
    expect(MockEventSource.instances.length).toBe(2);
  });

  it("force-closes the EventSource on the 'online' event", () => {
    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    const original = MockEventSource.instances[0];
    expect(original.readyState).toBe(MockEventSource.OPEN);

    act(() => {
      window.dispatchEvent(new Event("online"));
    });

    expect(original.readyState).toBe(MockEventSource.CLOSED);
    expect(MockEventSource.instances.length).toBe(2);
  });

  it("removes the 'online' listener on unmount", () => {
    const removeSpy = vi.spyOn(window, "removeEventListener");
    const { unmount } = renderHook(() => useSSE(), {
      wrapper: makeWrapper(queryClient),
    });

    unmount();

    const removedOnline = removeSpy.mock.calls.some((call) => call[0] === "online");
    expect(removedOnline).toBe(true);
  });
});

// --- Paginated-cache patching ---------------------------------------------
// Filter-aware in-place upsert/remove against the four filtered cache shapes
// the SSE handler now patches instead of broadly invalidating.

interface SeededIssue {
  issue_id: string;
  version: number;
  timestamp: string;
  creation_time: string;
  issue: {
    type: string;
    title: string;
    description: string;
    creator: string;
    status: { key: string };
    project_id?: string;
    progress: string;
    dependencies: never[];
    patches: never[];
    labels: never[];
  };
}

function seedIssue(
  issueId: string,
  status = "open",
  overrides: Partial<SeededIssue["issue"]> = {},
): SeededIssue {
  return {
    issue_id: issueId,
    version: 1,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    issue: {
      type: "task",
      title: `Issue ${issueId}`,
      description: "desc",
      creator: "alice",
      status: { key: status },
      progress: "",
      dependencies: [],
      patches: [],
      labels: [],
      ...overrides,
    },
  };
}

describe("useSSE paginated-cache patching", () => {
  let queryClient: QueryClient;

  beforeEach(() => {
    queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
  });

  it("removes an issue from ['paginatedIssues', {status:'open'}, 'sort', ...] when its status flips out of the filter", () => {
    const filters = { status: "open" };
    const key = ["paginatedIssues", filters, "sort", "project_status_time_desc"];
    queryClient.setQueryData(key, {
      pages: [
        {
          issues: [seedIssue("i-1", "open"), seedIssue("i-2", "open")],
          next_cursor: null,
        },
      ],
      pageParams: [undefined],
    });

    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    const es = MockEventSource.instances[0];

    act(() => {
      es.dispatch("issue_updated", {
        entity_type: "issue",
        entity_id: "i-1",
        version: 2,
        timestamp: "2026-01-01T00:00:00Z",
        entity: { ...seedIssue("i-1", "in-progress"), version: 2 },
      });
    });

    const cached = queryClient.getQueryData<{
      pages: Array<{ issues: Array<{ issue_id: string }> }>;
    }>(key);
    expect(cached?.pages[0].issues.map((i) => i.issue_id)).toEqual(["i-2"]);
  });

  it("appends a newly-created issue to the board-bulk single-response cache", () => {
    const filters = {};
    const key = ["paginatedIssues", filters, "board-bulk", "project_status_time_desc"];
    queryClient.setQueryData(key, { issues: [], next_cursor: null });

    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    const es = MockEventSource.instances[0];

    act(() => {
      es.dispatch("issue_created", {
        entity_type: "issue",
        entity_id: "i-new",
        version: 1,
        timestamp: "2026-01-01T00:00:00Z",
        entity: seedIssue("i-new", "open"),
      });
    });

    const cached = queryClient.getQueryData<{
      issues: Array<{ issue_id: string }>;
    }>(key);
    expect(cached?.issues.map((i) => i.issue_id)).toEqual(["i-new"]);
  });

  it("updates and removes issues in the per-cell expanded ['depth', n] array-of-pages cache based on filter match", () => {
    const filters = { project_id: "P1", status: "open" };
    const key = ["paginatedIssues", filters, "depth", 2];
    queryClient.setQueryData(key, [
      {
        issues: [
          seedIssue("i-1", "open", { project_id: "P1" }),
          seedIssue("i-2", "open", { project_id: "P1" }),
        ],
        next_cursor: null,
      },
      {
        issues: [seedIssue("i-3", "open", { project_id: "P1" })],
        next_cursor: null,
      },
    ]);

    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    const es = MockEventSource.instances[0];

    // Update-in-place: i-1 stays "open" but title changes.
    act(() => {
      es.dispatch("issue_updated", {
        entity_type: "issue",
        entity_id: "i-1",
        version: 2,
        timestamp: "2026-01-01T00:00:00Z",
        entity: {
          ...seedIssue("i-1", "open", { project_id: "P1", title: "Renamed" }),
          version: 2,
        },
      });
    });

    let cached =
      queryClient.getQueryData<
        Array<{ issues: Array<{ issue_id: string; issue: { title: string } }> }>
      >(key);
    expect(cached?.[0].issues[0].issue.title).toBe("Renamed");

    // Removal: i-2's new status flips out of the filter.
    act(() => {
      es.dispatch("issue_updated", {
        entity_type: "issue",
        entity_id: "i-2",
        version: 2,
        timestamp: "2026-01-01T00:00:00Z",
        entity: {
          ...seedIssue("i-2", "closed", { project_id: "P1" }),
          version: 2,
        },
      });
    });

    cached =
      queryClient.getQueryData<
        Array<{ issues: Array<{ issue_id: string; issue: { title: string } }> }>
      >(key);
    expect(cached?.[0].issues.map((i) => i.issue_id)).toEqual(["i-1"]);
    expect(cached?.[1].issues.map((i) => i.issue_id)).toEqual(["i-3"]);
  });

  it("prepends a newly-created session to ['paginatedSessions', filters] (InfiniteData)", () => {
    const key = ["paginatedSessions", {}];
    queryClient.setQueryData(key, {
      pages: [{ sessions: [], next_cursor: null }],
      pageParams: [undefined],
    });

    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    const es = MockEventSource.instances[0];

    act(() => {
      es.dispatch("session_created", {
        entity_type: "session",
        entity_id: "s-new",
        version: 1,
        timestamp: "2026-01-01T00:00:00Z",
        entity: makeSessionRecord("s-new", "i-1"),
      });
    });

    const cached = queryClient.getQueryData<{
      pages: Array<{ sessions: Array<{ session_id: string }> }>;
    }>(key);
    expect(cached?.pages[0].sessions.map((s) => s.session_id)).toEqual(["s-new"]);
  });

  it("does not throw or write into the ['issues', id, 'comments'] infinite cache on issue_updated (P9 regression)", () => {
    // The IssueDetail Comments tab keys on ["issues", id, "comments"], whose
    // InfiniteData<ListCommentsResponse> shape has no `.issues` field. The
    // prior bug was that the `["issues"]` upsert prefix-matched into this
    // cache and threw on every issue_* SSE event. The new predicate restricts
    // the upsert to the bare `["issues"]` flat list.
    const commentsKey = ["issues", "i-1", "comments"];
    const commentsCache = {
      pages: [{ comments: [{ sequence: 1n, body: "hi" }], next_before_sequence: null }],
      pageParams: [undefined],
    };
    queryClient.setQueryData(commentsKey, commentsCache);

    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    const es = MockEventSource.instances[0];

    const consoleErrorSpy = vi.spyOn(console, "error").mockImplementation(() => {});

    expect(() => {
      act(() => {
        es.dispatch("issue_updated", {
          entity_type: "issue",
          entity_id: "i-1",
          version: 2,
          timestamp: "2026-01-01T00:00:00Z",
          entity: { ...seedIssue("i-1", "open"), version: 2 },
        });
      });
    }).not.toThrow();

    expect(consoleErrorSpy).not.toHaveBeenCalled();
    // The comments cache is unchanged.
    expect(queryClient.getQueryData(commentsKey)).toBe(commentsCache);

    consoleErrorSpy.mockRestore();
  });
});
