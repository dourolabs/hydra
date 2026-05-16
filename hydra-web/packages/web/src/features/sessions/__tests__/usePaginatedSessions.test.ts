// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor, act } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type {
  ListSessionsResponse,
  SearchSessionsQuery,
  SessionSummaryRecord,
} from "@hydra/api";

const mockListSessions = vi.fn();

vi.mock("../../../api/client", () => ({
  apiClient: {
    listSessions: (...args: unknown[]) => mockListSessions(...args),
  },
}));

function makeWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return ({ children }: { children: React.ReactNode }) =>
    React.createElement(QueryClientProvider, { client: queryClient }, children);
}

function rec(id: string): SessionSummaryRecord {
  return {
    session_id: id,
    version: 1n,
    timestamp: "2026-03-15T10:00:00.000Z",
    session: {
      prompt: "do the thing",
      creator: "swe",
      status: "running",
      start_time: "2026-03-15T10:00:00.000Z",
      end_time: null,
    },
  } as SessionSummaryRecord;
}

function page(
  sessions: SessionSummaryRecord[],
  nextCursor: string | null = null,
  totalCount: bigint | null = null,
): ListSessionsResponse {
  return {
    sessions,
    next_cursor: nextCursor,
    total_count: totalCount,
  };
}

const { usePaginatedSessions, useSessionCount } = await import(
  "../usePaginatedSessions"
);

describe("usePaginatedSessions", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("requests the first page with limit=50 and no cursor, exposing next_cursor", async () => {
    mockListSessions.mockResolvedValueOnce(page([rec("t-1"), rec("t-2")], "cur-2"));

    const { result } = renderHook(() => usePaginatedSessions({}), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.data?.pages.length).toBe(1);
    });

    expect(mockListSessions).toHaveBeenCalledTimes(1);
    const firstCallArg = mockListSessions.mock.calls[0][0] as Partial<SearchSessionsQuery>;
    expect(firstCallArg.limit).toBe(50);
    expect(firstCallArg.cursor).toBeUndefined();
    expect(firstCallArg.status).toBeUndefined();

    expect(result.current.data?.pages[0].sessions.map((s) => s.session_id)).toEqual([
      "t-1",
      "t-2",
    ]);
    expect(result.current.hasNextPage).toBe(true);
  });

  it("fetchNextPage passes the previous page's next_cursor", async () => {
    mockListSessions
      .mockResolvedValueOnce(page([rec("t-1")], "cursor-page-2"))
      .mockResolvedValueOnce(page([rec("t-2")], null));

    const { result } = renderHook(() => usePaginatedSessions({}), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.data?.pages.length).toBe(1);
    });

    await act(async () => {
      await result.current.fetchNextPage();
    });

    await waitFor(() => {
      expect(result.current.data?.pages.length).toBe(2);
    });

    expect(mockListSessions).toHaveBeenCalledTimes(2);
    const secondCallArg = mockListSessions.mock.calls[1][0] as Partial<SearchSessionsQuery>;
    expect(secondCallArg.cursor).toBe("cursor-page-2");
    expect(secondCallArg.limit).toBe(50);

    expect(result.current.hasNextPage).toBe(false);
  });

  it("changing the status filter triggers a separate cached fetch (different cache key)", async () => {
    mockListSessions.mockImplementation((query: Partial<SearchSessionsQuery>) => {
      if (query.status === "running") {
        return Promise.resolve(page([rec("active-1")]));
      }
      return Promise.resolve(page([rec("any-1"), rec("any-2")]));
    });

    const wrapper = makeWrapper();
    const { result, rerender } = renderHook(
      ({ status }: { status: string | null }) =>
        usePaginatedSessions({ status }),
      { wrapper, initialProps: { status: null as string | null } },
    );

    await waitFor(() => {
      expect(result.current.data?.pages[0].sessions.length).toBe(2);
    });

    expect(mockListSessions).toHaveBeenCalledTimes(1);
    expect(
      (mockListSessions.mock.calls[0][0] as Partial<SearchSessionsQuery>).status,
    ).toBeUndefined();

    rerender({ status: "running" });

    await waitFor(() => {
      expect(result.current.data?.pages[0].sessions[0].session_id).toBe("active-1");
    });

    expect(mockListSessions).toHaveBeenCalledTimes(2);
    expect(
      (mockListSessions.mock.calls[1][0] as Partial<SearchSessionsQuery>).status,
    ).toBe("running");
  });

  it("omits the status field from the request when filter is null/empty", async () => {
    mockListSessions.mockResolvedValueOnce(page([]));

    const { result } = renderHook(
      () => usePaginatedSessions({ status: null }),
      { wrapper: makeWrapper() },
    );

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });

    const call = mockListSessions.mock.calls[0][0] as Partial<SearchSessionsQuery>;
    expect(call).not.toHaveProperty("status");
  });
});

describe("useSessionCount", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("requests count=true and limit=0, returning total_count as a number", async () => {
    mockListSessions.mockResolvedValueOnce(page([], null, 1234n));

    const { result } = renderHook(() => useSessionCount({}), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.data).toBe(1234);
    });

    const call = mockListSessions.mock.calls[0][0] as Partial<SearchSessionsQuery>;
    expect(call.count).toBe(true);
    expect(call.limit).toBe(0);
  });

  it("returns 0 when total_count is missing/null", async () => {
    mockListSessions.mockResolvedValueOnce(page([], null, null));

    const { result } = renderHook(() => useSessionCount({}), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.data).toBe(0);
    });
  });

  it("includes status in the request when filter is set", async () => {
    mockListSessions.mockResolvedValueOnce(page([], null, 7n));

    const { result } = renderHook(
      () => useSessionCount({ status: "running" }),
      { wrapper: makeWrapper() },
    );

    await waitFor(() => {
      expect(result.current.data).toBe(7);
    });

    const call = mockListSessions.mock.calls[0][0] as Partial<SearchSessionsQuery>;
    expect(call.status).toBe("running");
  });
});
