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
    expect(wasInvalidated(invalidateSpy, ["chatRelated", "refers_to"])).toBe(true);
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
