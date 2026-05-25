import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type { DocumentSummaryRecord } from "@hydra/api";

const mockListRelations = vi.fn();
const mockListDocuments = vi.fn();

vi.mock("../../api/client", () => ({
  apiClient: {
    listRelations: (...args: unknown[]) => mockListRelations(...args),
    listDocuments: (...args: unknown[]) => mockListDocuments(...args),
  },
}));

function makeWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return ({ children }: { children: React.ReactNode }) =>
    React.createElement(QueryClientProvider, { client: queryClient }, children);
}

function makeDoc(id: string): DocumentSummaryRecord {
  return {
    document_id: id,
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    document: {
      title: `Doc ${id}`,
      path: `path/${id}`,
    },
  } as DocumentSummaryRecord;
}

const { useIssueDocuments } = await import("./useIssueDocuments");

describe("useIssueDocuments", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("queries has-document relations and resolves the linked documents", async () => {
    mockListRelations.mockResolvedValue({
      relations: [
        { source_id: "i-1", target_id: "d-a", rel_type: "has-document" },
        { source_id: "i-1", target_id: "d-b", rel_type: "has-document" },
      ],
    });
    mockListDocuments.mockResolvedValue({
      documents: [makeDoc("d-a"), makeDoc("d-b")],
    });

    const { result } = renderHook(() => useIssueDocuments("i-1"), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.data).toHaveLength(2);
    });

    expect(mockListRelations).toHaveBeenCalledWith({
      source_id: "i-1",
      rel_type: "has-document",
    });
    expect(mockListDocuments).toHaveBeenCalledWith({ ids: "d-a,d-b", limit: 2 });
    expect(result.current.data.map((d) => d.document_id)).toEqual(["d-a", "d-b"]);
    expect(result.current.error).toBeNull();
  });

  it("filters and orders the response by documentIds when listDocuments returns extras or different order", async () => {
    mockListRelations.mockResolvedValue({
      relations: [
        { source_id: "i-1", target_id: "d-b", rel_type: "has-document" },
        { source_id: "i-1", target_id: "d-a", rel_type: "has-document" },
      ],
    });
    mockListDocuments.mockResolvedValue({
      documents: [makeDoc("d-a"), makeDoc("d-b"), makeDoc("d-c")],
    });

    const { result } = renderHook(() => useIssueDocuments("i-1"), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.data).toHaveLength(2);
    });

    expect(result.current.data.map((d) => d.document_id)).toEqual(["d-b", "d-a"]);
  });

  it("returns an empty array and does not call listDocuments when no relations exist", async () => {
    mockListRelations.mockResolvedValue({ relations: [] });

    const { result } = renderHook(() => useIssueDocuments("i-empty"), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });

    expect(result.current.data).toEqual([]);
    expect(mockListRelations).toHaveBeenCalled();
    expect(mockListDocuments).not.toHaveBeenCalled();
  });

  it("does not fire any query when issueId is empty", async () => {
    const { result } = renderHook(() => useIssueDocuments(""), {
      wrapper: makeWrapper(),
    });

    await new Promise((r) => setTimeout(r, 50));

    expect(result.current.data).toEqual([]);
    expect(mockListRelations).not.toHaveBeenCalled();
    expect(mockListDocuments).not.toHaveBeenCalled();
  });

  it("propagates errors from listRelations", async () => {
    mockListRelations.mockRejectedValue(new Error("rel fail"));

    const { result } = renderHook(() => useIssueDocuments("i-1"), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.error).not.toBeNull();
    });

    expect((result.current.error as Error).message).toBe("rel fail");
    expect(mockListDocuments).not.toHaveBeenCalled();
  });

  it("does not leak the previous issue's documents when issueId changes to one with no documents", async () => {
    // Issue i-1 has a linked doc; issue i-2 has none. Re-rendering the hook
    // with i-2 must immediately show an empty array — no stale placeholder
    // from i-1's cache.
    mockListRelations.mockImplementation(({ source_id }: { source_id: string }) => {
      if (source_id === "i-1") {
        return Promise.resolve({
          relations: [
            { source_id: "i-1", target_id: "d-a", rel_type: "has-document" },
          ],
        });
      }
      return Promise.resolve({ relations: [] });
    });
    mockListDocuments.mockResolvedValue({ documents: [makeDoc("d-a")] });

    // Share one QueryClient across renders so cached i-1 data is available
    // when we switch to i-2 — this is what makes the placeholder fire in prod.
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const wrapper = ({ children }: { children: React.ReactNode }) =>
      React.createElement(QueryClientProvider, { client: queryClient }, children);

    const { result, rerender } = renderHook(
      ({ id }: { id: string }) => useIssueDocuments(id),
      { wrapper, initialProps: { id: "i-1" } },
    );

    await waitFor(() => {
      expect(result.current.data.map((d) => d.document_id)).toEqual(["d-a"]);
    });

    rerender({ id: "i-2" });

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });

    expect(result.current.data).toEqual([]);
  });
});
