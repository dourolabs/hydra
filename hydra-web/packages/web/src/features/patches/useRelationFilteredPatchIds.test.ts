// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type { Filter } from "../filters";

const mockListRelations = vi.fn();
const mockGetSession = vi.fn();

vi.mock("../../api/client", () => ({
  apiClient: {
    listRelations: (...args: unknown[]) => mockListRelations(...args),
    getSession: (...args: unknown[]) => mockGetSession(...args),
  },
}));

function makeWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return ({ children }: { children: React.ReactNode }) =>
    React.createElement(QueryClientProvider, { client: queryClient }, children);
}

function mkFilter(id: string, values: string[]): Filter {
  return { _uid: `t:${id}`, id, op: "in", values };
}

const { useRelationFilteredPatchIds } = await import(
  "./useRelationFilteredPatchIds"
);

describe("useRelationFilteredPatchIds", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("returns patchIds=null and is not loading when no relation filter is active", async () => {
    const { result } = renderHook(() => useRelationFilteredPatchIds([]), {
      wrapper: makeWrapper(),
    });

    expect(result.current.patchIds).toBeNull();
    expect(result.current.isLoading).toBe(false);
    expect(mockListRelations).not.toHaveBeenCalled();
    expect(mockGetSession).not.toHaveBeenCalled();
  });

  it("resolves a relatedIssue filter via /v1/relations has-patch edges", async () => {
    mockListRelations.mockResolvedValueOnce({
      relations: [
        { source_id: "i-aaa", target_id: "p-1", rel_type: "has-patch" },
        { source_id: "i-aaa", target_id: "p-2", rel_type: "has-patch" },
      ],
    });

    const { result } = renderHook(
      () => useRelationFilteredPatchIds([mkFilter("relatedIssue", ["i-aaa"])]),
      { wrapper: makeWrapper() },
    );

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });

    expect(mockListRelations).toHaveBeenCalledWith({
      source_ids: "i-aaa",
      rel_type: "has-patch",
    });
    expect(new Set(result.current.patchIds ?? [])).toEqual(
      new Set(["p-1", "p-2"]),
    );
  });

  it("resolves a relatedSession filter by 2-hop session→issue→has-patch lookup", async () => {
    // Hop 1: getSession returns a session whose spawned_from issue is i-x.
    mockGetSession.mockResolvedValueOnce({
      session_id: "s-1",
      version: 1n,
      timestamp: "2026-01-01T00:00:00Z",
      session: {
        creator: "swe",
        prompt: "",
        status: "running",
        spawned_from: "i-x",
        start_time: null,
        end_time: null,
      },
    });

    // Hop 2: the has-patch lookup for i-x returns one patch.
    mockListRelations.mockResolvedValueOnce({
      relations: [
        { source_id: "i-x", target_id: "p-7", rel_type: "has-patch" },
      ],
    });

    const { result } = renderHook(
      () =>
        useRelationFilteredPatchIds([mkFilter("relatedSession", ["s-1"])]),
      { wrapper: makeWrapper() },
    );

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });

    expect(mockGetSession).toHaveBeenCalledWith("s-1");
    expect(mockListRelations).toHaveBeenCalledWith({
      source_ids: "i-x",
      rel_type: "has-patch",
    });
    expect(result.current.patchIds).toEqual(["p-7"]);
  });

  it("returns an empty array (not null) when the session has no spawned_from", async () => {
    mockGetSession.mockResolvedValueOnce({
      session_id: "s-1",
      version: 1n,
      timestamp: "2026-01-01T00:00:00Z",
      session: {
        creator: "swe",
        prompt: "",
        status: "running",
        spawned_from: null,
        start_time: null,
        end_time: null,
      },
    });

    const { result } = renderHook(
      () =>
        useRelationFilteredPatchIds([mkFilter("relatedSession", ["s-1"])]),
      { wrapper: makeWrapper() },
    );

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });

    // No issues to look up → hop 2 must NOT fire.
    expect(mockListRelations).not.toHaveBeenCalled();
    expect(result.current.patchIds).toEqual([]);
  });

  it("intersects the patch-id sets across multiple relation filters (AND)", async () => {
    // relatedIssue → p-1, p-2, p-3
    mockListRelations.mockResolvedValueOnce({
      relations: [
        { source_id: "i-a", target_id: "p-1", rel_type: "has-patch" },
        { source_id: "i-a", target_id: "p-2", rel_type: "has-patch" },
        { source_id: "i-a", target_id: "p-3", rel_type: "has-patch" },
      ],
    });
    // relatedSession → session s-1 → issue i-b → p-2, p-3
    mockGetSession.mockResolvedValueOnce({
      session_id: "s-1",
      version: 1n,
      timestamp: "2026-01-01T00:00:00Z",
      session: {
        creator: "swe",
        prompt: "",
        status: "running",
        spawned_from: "i-b",
        start_time: null,
        end_time: null,
      },
    });
    mockListRelations.mockResolvedValueOnce({
      relations: [
        { source_id: "i-b", target_id: "p-2", rel_type: "has-patch" },
        { source_id: "i-b", target_id: "p-3", rel_type: "has-patch" },
      ],
    });

    const { result } = renderHook(
      () =>
        useRelationFilteredPatchIds([
          mkFilter("relatedIssue", ["i-a"]),
          mkFilter("relatedSession", ["s-1"]),
        ]),
      { wrapper: makeWrapper() },
    );

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });

    // p-1 ∉ session-side set, so the AND-intersection drops it.
    expect(new Set(result.current.patchIds ?? [])).toEqual(
      new Set(["p-2", "p-3"]),
    );
  });
});
