import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type { PatchVersionRecord } from "@metis/api";

// --- Mocks ---

const mockGetPatch = vi.fn();

vi.mock("../../api/client", () => ({
  apiClient: {
    getPatch: (...args: unknown[]) => mockGetPatch(...args),
  },
}));

// --- Helpers ---

function makeWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return ({ children }: { children: React.ReactNode }) =>
    React.createElement(QueryClientProvider, { client: queryClient }, children);
}

function makePatch(id: string): PatchVersionRecord {
  return {
    patch_id: id,
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    patch: {
      title: `Patch ${id}`,
      description: "",
      diff: "",
      status: "Open",
      is_automatic_backup: false,
      creator: "alice",
      reviews: [],
      review_summary: { count: 0, approved: false },
      service_repo_name: "test-repo",
    },
  } as PatchVersionRecord;
}

// --- Import after mocks ---
const { usePatchesByIds } = await import("./usePatchesByIds");

// --- Tests ---

describe("usePatchesByIds", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("returns empty array for empty patchIds", async () => {
    const { result } = renderHook(() => usePatchesByIds([]), {
      wrapper: makeWrapper(),
    });

    // Wait a tick to ensure no queries fire
    await new Promise((r) => setTimeout(r, 50));

    expect(result.current.data).toEqual([]);
    expect(result.current.isLoading).toBe(false);
    expect(mockGetPatch).not.toHaveBeenCalled();
  });

  it("fetches and returns all patches", async () => {
    const patch1 = makePatch("p-1");
    const patch2 = makePatch("p-2");

    mockGetPatch.mockImplementation((id: string) => {
      if (id === "p-1") return Promise.resolve(patch1);
      if (id === "p-2") return Promise.resolve(patch2);
      return Promise.reject(new Error("Unknown patch"));
    });

    const { result } = renderHook(() => usePatchesByIds(["p-1", "p-2"]), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.data).toHaveLength(2);
    });

    expect(mockGetPatch).toHaveBeenCalledWith("p-1");
    expect(mockGetPatch).toHaveBeenCalledWith("p-2");

    const ids = result.current.data.map((p) => p.patch_id);
    expect(ids).toContain("p-1");
    expect(ids).toContain("p-2");
    expect(result.current.error).toBeNull();
  });

  it("sets error when a getPatch call fails", async () => {
    const patch1 = makePatch("p-ok");

    mockGetPatch.mockImplementation((id: string) => {
      if (id === "p-ok") return Promise.resolve(patch1);
      return Promise.reject(new Error("Not found"));
    });

    const { result } = renderHook(() => usePatchesByIds(["p-fail", "p-ok"]), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.error).not.toBeNull();
    });

    expect(result.current.error).toBeInstanceOf(Error);
    // The successful patch should still be in data
    const ids = result.current.data.map((p) => p.patch_id);
    expect(ids).toContain("p-ok");
  });

  it("stabilizes query keys via sorting (same queries for different order)", async () => {
    const patch1 = makePatch("p-a");
    const patch2 = makePatch("p-b");

    mockGetPatch.mockImplementation((id: string) => {
      if (id === "p-a") return Promise.resolve(patch1);
      if (id === "p-b") return Promise.resolve(patch2);
      return Promise.reject(new Error("Unknown"));
    });

    const wrapper = makeWrapper();

    // Render with ["p-b", "p-a"] — hook should sort to ["p-a", "p-b"]
    const { result, rerender } = renderHook(
      ({ ids }: { ids: string[] }) => usePatchesByIds(ids),
      { wrapper, initialProps: { ids: ["p-b", "p-a"] } },
    );

    await waitFor(() => {
      expect(result.current.data).toHaveLength(2);
    });

    const callCount = mockGetPatch.mock.calls.length;

    // Re-render with ["p-a", "p-b"] — same sorted key, no new queries
    rerender({ ids: ["p-a", "p-b"] });

    // Wait a tick for any potential refetches
    await new Promise((r) => setTimeout(r, 50));

    // No additional getPatch calls should have been made
    expect(mockGetPatch.mock.calls.length).toBe(callCount);
  });
});
