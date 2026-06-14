// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type { ListPatchesResponse, PatchSummaryRecord, SearchPatchesQuery } from "@hydra/api";

const mockListPatches = vi.fn();

vi.mock("../../../api/client", () => ({
  apiClient: {
    listPatches: (...args: unknown[]) => mockListPatches(...args),
  },
}));

function makeWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return ({ children }: { children: React.ReactNode }) =>
    React.createElement(QueryClientProvider, { client: queryClient }, children);
}

function rec(id: string): PatchSummaryRecord {
  return {
    patch_id: id,
    version: 1n,
    timestamp: "2026-03-15T10:00:00.000Z",
    creation_time: "2026-03-15T10:00:00.000Z",
    patch: {
      title: `Patch ${id}`,
      description: "",
      diff: "",
      status: "open",
      is_automatic_backup: false,
      creator: "alice",
      reviews: [],
      review_summary: { count: 0, approved: false },
      service_repo_name: "test-repo",
    },
  } as PatchSummaryRecord;
}

function page(
  patches: PatchSummaryRecord[],
  nextCursor: string | null = null,
  totalCount: bigint | null = null,
): ListPatchesResponse {
  return {
    patches,
    next_cursor: nextCursor,
    total_count: totalCount,
  };
}

const { usePatchCount } = await import("../usePaginatedPatches");

describe("usePatchCount", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("requests count=true and limit=0, returning total_count as a number", async () => {
    mockListPatches.mockResolvedValueOnce(page([], null, 1234n));

    const { result } = renderHook(() => usePatchCount({}), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.data).toBe(1234);
    });

    const call = mockListPatches.mock.calls[0][0] as Partial<SearchPatchesQuery>;
    expect(call.count).toBe(true);
    expect(call.limit).toBe(0);
  });

  it("returns 0 when total_count is missing/null", async () => {
    mockListPatches.mockResolvedValueOnce(page([rec("p-1")], null, null));

    const { result } = renderHook(() => usePatchCount({}), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.data).toBe(0);
    });
  });

  it("includes status and q in the request when filters are set", async () => {
    mockListPatches.mockResolvedValueOnce(page([], null, 7n));

    const { result } = renderHook(() => usePatchCount({ status: ["open"], q: "hello" }), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.data).toBe(7);
    });

    const call = mockListPatches.mock.calls[0][0] as Partial<SearchPatchesQuery>;
    expect(call.status).toEqual(["open"]);
    expect(call.q).toBe("hello");
    expect(call.count).toBe(true);
    expect(call.limit).toBe(0);
  });

  it("omits status when the filter is an empty array", async () => {
    mockListPatches.mockResolvedValueOnce(page([], null, 0n));

    const { result } = renderHook(() => usePatchCount({ status: [] }), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });

    const call = mockListPatches.mock.calls[0][0] as Partial<SearchPatchesQuery>;
    expect(call.status).toBeUndefined();
  });

  it("does not fire the request when enabled=false", async () => {
    const { result } = renderHook(() => usePatchCount({}, false), {
      wrapper: makeWrapper(),
    });

    // Wait a tick to ensure no queries fire
    await new Promise((r) => setTimeout(r, 50));

    expect(mockListPatches).not.toHaveBeenCalled();
    expect(result.current.data).toBeUndefined();
  });
});
