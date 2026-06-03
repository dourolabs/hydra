// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type { Filter } from "../../filters";

const mockListRelations = vi.fn();

vi.mock("../../../api/client", () => ({
  apiClient: {
    listRelations: (...args: unknown[]) => mockListRelations(...args),
  },
}));

function makeWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return ({ children }: { children: React.ReactNode }) =>
    React.createElement(QueryClientProvider, { client: queryClient }, children);
}

const { useRelationFilteredSessionIds } = await import(
  "../useRelationFilteredSessionIds"
);

function f(id: string, values: string[]): Filter {
  return { _uid: `uid-${id}`, id, op: "in", values };
}

describe("useRelationFilteredSessionIds", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("returns null patchIssueIds when no relatedPatch filter is active", () => {
    const { result } = renderHook(
      () => useRelationFilteredSessionIds([f("status", ["running"])]),
      { wrapper: makeWrapper() },
    );
    expect(result.current).toEqual({ patchIssueIds: null, isLoading: false });
    expect(mockListRelations).not.toHaveBeenCalled();
  });

  it("ignores relatedPatch filters with no values", () => {
    const { result } = renderHook(
      () => useRelationFilteredSessionIds([f("relatedPatch", [])]),
      { wrapper: makeWrapper() },
    );
    expect(result.current).toEqual({ patchIssueIds: null, isLoading: false });
    expect(mockListRelations).not.toHaveBeenCalled();
  });

  it("issues a single /v1/relations 2-hop and returns the source issue ids", async () => {
    mockListRelations.mockResolvedValueOnce({
      relations: [
        { source_id: "i-1", target_id: "p-aa", rel_type: "has-patch" },
        { source_id: "i-2", target_id: "p-bb", rel_type: "has-patch" },
        { source_id: "i-1", target_id: "p-bb", rel_type: "has-patch" },
      ],
    });

    const { result } = renderHook(
      () =>
        useRelationFilteredSessionIds([f("relatedPatch", ["p-aa", "p-bb"])]),
      { wrapper: makeWrapper() },
    );

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });

    expect(mockListRelations).toHaveBeenCalledTimes(1);
    expect(mockListRelations).toHaveBeenCalledWith({
      target_ids: "p-aa,p-bb",
      rel_type: "has-patch",
    });

    const ids = (result.current.patchIssueIds ?? []).slice().sort();
    expect(ids).toEqual(["i-1", "i-2"]);
  });

  it("returns an empty array when the 2-hop finds no related issues", async () => {
    mockListRelations.mockResolvedValueOnce({ relations: [] });

    const { result } = renderHook(
      () => useRelationFilteredSessionIds([f("relatedPatch", ["p-x"])]),
      { wrapper: makeWrapper() },
    );

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });

    expect(result.current.patchIssueIds).toEqual([]);
  });
});
