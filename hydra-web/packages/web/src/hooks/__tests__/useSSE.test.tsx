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
  (globalThis as unknown as { EventSource: typeof MockEventSource }).EventSource =
    MockEventSource;
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
      status: "Open",
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
      deleted: false,
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

  it("invalidates ['sessions', 'active'] on session_created with spawned_from set", () => {
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

    // The sidebar's useActiveSessions hook uses key ["sessions", "active", limit].
    // The bug was that the spawnedFrom branch never invalidated this key — only
    // ["sessions", "all"] — so the sidebar never refetched. Verify the explicit
    // active-sessions invalidate at the common level now covers it.
    expect(wasInvalidated(invalidateSpy, ["sessions", "active"])).toBe(true);
    // The pre-existing activeCount invalidate must still fire alongside it.
    expect(wasInvalidated(invalidateSpy, ["sessions", "activeCount"])).toBe(true);
  });

  it("invalidates ['sessions', 'active'] on session_updated with spawned_from set", () => {
    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    const es = MockEventSource.instances[0];

    act(() => {
      es.dispatch("session_updated", {
        entity_type: "session",
        entity_id: "s-1",
        version: 2,
        timestamp: "2026-01-01T00:00:00Z",
        entity: makeSessionRecord("s-1", "i-1"),
      });
    });

    // Status transitions (e.g., running → complete) should also refresh the
    // sidebar so terminated sessions disappear via the server's status filter.
    expect(wasInvalidated(invalidateSpy, ["sessions", "active"])).toBe(true);
  });

  it("invalidates ['sessions', 'active'] on session_updated without spawned_from (else branch)", () => {
    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    const es = MockEventSource.instances[0];

    act(() => {
      es.dispatch("session_updated", {
        entity_type: "session",
        entity_id: "s-2",
        version: 1,
        timestamp: "2026-01-01T00:00:00Z",
        entity: makeSessionRecord("s-2"),
      });
    });

    // The else branch broadly invalidates ["sessions"] (prefix match covers
    // ["sessions", "active"]); the explicit invalidate also runs. Either way,
    // the active-sessions cache must be invalidated.
    expect(wasInvalidated(invalidateSpy, ["sessions", "active"])).toBe(true);
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

  it("invalidates ['sessionEvents', sid] and ['sessionsByConversation'] on session_event_created", () => {
    renderHook(() => useSSE(), { wrapper: makeWrapper(queryClient) });
    const es = MockEventSource.instances[0];

    act(() => {
      es.dispatch("session_event_created", {
        entity_type: "session_event",
        entity_id: "t-abc",
        version: 4,
        timestamp: "2026-05-23T17:00:00Z",
        // SessionEvent JSON payload (not a SessionSummaryRecord) — the new
        // arm must not feed this to upsertBatchSession.
        entity: {
          type: "user_message",
          content: "hi",
          timestamp: "2026-05-23T17:00:00Z",
        },
      });
    });

    // The per-session events query is invalidated so the chat page refetches
    // just that session's SessionEvent log.
    expect(wasInvalidated(invalidateSpy, ["sessionEvents", "t-abc"])).toBe(true);
    // The conversation→sessions index is invalidated broadly (SSE payload
    // doesn't carry conversation_id, so prefix-match covers any open chat).
    expect(wasInvalidated(invalidateSpy, ["sessionsByConversation"])).toBe(true);
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
    const scheduled = setTimeoutSpy.mock.calls
      .slice(callsBefore)
      .find((c) => {
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

    const { delay } = fireErrorAndCaptureDelay(
      setTimeoutSpy,
      MockEventSource.instances[0],
    );
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
    const first = fireErrorAndCaptureDelay(
      setTimeoutSpy,
      MockEventSource.instances[0],
    );

    // Drive the scheduled reconnect inline (instead of waiting wall-clock)
    // so the second instance comes up and we can fire a second onerror.
    act(() => {
      first.reconnect();
    });
    expect(MockEventSource.instances.length).toBe(2);

    const second = fireErrorAndCaptureDelay(
      setTimeoutSpy,
      MockEventSource.instances[1],
    );

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

    const removedOnline = removeSpy.mock.calls.some(
      (call) => call[0] === "online",
    );
    expect(removedOnline).toBe(true);
  });
});
