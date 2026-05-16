import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type { PatchSummaryRecord } from "@hydra/api";

const mockListRelations = vi.fn();
const mockListPatches = vi.fn();

vi.mock("../../api/client", () => ({
  apiClient: {
    listRelations: (...args: unknown[]) => mockListRelations(...args),
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

function makePatch(id: string): PatchSummaryRecord {
  return {
    patch_id: id,
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    patch: {
      title: `Patch ${id}`,
      status: "Open",
      creator: "alice",
      is_automatic_backup: false,
      review_summary: { count: 0, approved: false },
      service_repo_name: "test-repo",
    },
  } as PatchSummaryRecord;
}

const { useIssuePatches } = await import("./useIssuePatches");

describe("useIssuePatches", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("queries has-patch relations and resolves the linked patches", async () => {
    mockListRelations.mockResolvedValue({
      relations: [
        { source_id: "i-1", target_id: "p-a", rel_type: "has-patch" },
        { source_id: "i-1", target_id: "p-b", rel_type: "has-patch" },
      ],
    });
    mockListPatches.mockResolvedValue({
      patches: [makePatch("p-a"), makePatch("p-b")],
    });

    const { result } = renderHook(() => useIssuePatches("i-1"), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.data).toHaveLength(2);
    });

    expect(mockListRelations).toHaveBeenCalledWith({
      source_id: "i-1",
      rel_type: "has-patch",
    });
    expect(mockListPatches).toHaveBeenCalledWith({ ids: "p-a,p-b", limit: 2 });
    expect(result.current.data.map((p) => p.patch_id)).toEqual(["p-a", "p-b"]);
    expect(result.current.error).toBeNull();
  });

  it("filters and orders the response by patchIds when listPatches returns extras or different order", async () => {
    mockListRelations.mockResolvedValue({
      relations: [
        { source_id: "i-1", target_id: "p-b", rel_type: "has-patch" },
        { source_id: "i-1", target_id: "p-a", rel_type: "has-patch" },
      ],
    });
    // Mock-server returns the full patch list and ignores the `ids` filter.
    mockListPatches.mockResolvedValue({
      patches: [
        makePatch("p-a"),
        makePatch("p-b"),
        makePatch("p-c"),
        makePatch("p-d"),
      ],
    });

    const { result } = renderHook(() => useIssuePatches("i-1"), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.data).toHaveLength(2);
    });

    // Only the patches named in the relations should appear, in relations order.
    expect(result.current.data.map((p) => p.patch_id)).toEqual(["p-b", "p-a"]);
  });

  it("returns an empty array and does not call listPatches when no relations exist", async () => {
    mockListRelations.mockResolvedValue({ relations: [] });

    const { result } = renderHook(() => useIssuePatches("i-empty"), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });

    expect(result.current.data).toEqual([]);
    expect(mockListRelations).toHaveBeenCalled();
    expect(mockListPatches).not.toHaveBeenCalled();
  });

  it("does not fire any query when issueId is empty", async () => {
    const { result } = renderHook(() => useIssuePatches(""), {
      wrapper: makeWrapper(),
    });

    await new Promise((r) => setTimeout(r, 50));

    expect(result.current.data).toEqual([]);
    expect(mockListRelations).not.toHaveBeenCalled();
    expect(mockListPatches).not.toHaveBeenCalled();
  });

  it("propagates errors from listRelations", async () => {
    mockListRelations.mockRejectedValue(new Error("rel fail"));

    const { result } = renderHook(() => useIssuePatches("i-1"), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.error).not.toBeNull();
    });

    expect((result.current.error as Error).message).toBe("rel fail");
    expect(mockListPatches).not.toHaveBeenCalled();
  });
});
