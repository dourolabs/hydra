import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type { DocumentSummaryRecord } from "@hydra/api";

// --- Mocks ---

const mockListDocuments = vi.fn();

vi.mock("../../api/client", () => ({
  apiClient: {
    listDocuments: (...args: unknown[]) => mockListDocuments(...args),
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

function makeDocument(id: string): DocumentSummaryRecord {
  return {
    document_id: id,
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    document: {
      title: `Document ${id}`,
      path: `docs/${id}.md`,
    },
  } as DocumentSummaryRecord;
}

// --- Import after mocks ---
const { useDocumentsByIds } = await import("./useDocumentsByIds");

// --- Tests ---

describe("useDocumentsByIds", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("returns empty array for empty documentIds", async () => {
    const { result } = renderHook(() => useDocumentsByIds([]), {
      wrapper: makeWrapper(),
    });

    // Wait a tick to ensure no queries fire
    await new Promise((r) => setTimeout(r, 50));

    expect(result.current.data).toEqual([]);
    expect(result.current.isLoading).toBe(false);
    expect(mockListDocuments).not.toHaveBeenCalled();
  });

  it("fetches and returns all documents in input order", async () => {
    const doc1 = makeDocument("d-1");
    const doc2 = makeDocument("d-2");

    mockListDocuments.mockResolvedValue({ documents: [doc2, doc1] });

    const { result } = renderHook(() => useDocumentsByIds(["d-1", "d-2"]), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.data).toHaveLength(2);
    });

    expect(mockListDocuments).toHaveBeenCalledTimes(1);
    expect(mockListDocuments).toHaveBeenCalledWith({ ids: "d-1,d-2", limit: 2 });

    // Order matches the sorted-input order (the stable key) regardless of
    // server response order.
    expect(result.current.data.map((d) => d.document_id)).toEqual([
      "d-1",
      "d-2",
    ]);
    expect(result.current.error).toBeNull();
  });

  it("skips ids that are absent from the response", async () => {
    const doc1 = makeDocument("d-ok");
    // Server only returns d-ok; the requested d-missing was deleted.
    mockListDocuments.mockResolvedValue({ documents: [doc1] });

    const { result } = renderHook(
      () => useDocumentsByIds(["d-ok", "d-missing"]),
      { wrapper: makeWrapper() },
    );

    await waitFor(() => {
      expect(result.current.data).toHaveLength(1);
    });

    expect(result.current.data.map((d) => d.document_id)).toEqual(["d-ok"]);
    expect(result.current.error).toBeNull();
  });

  it("sets error when the listDocuments call fails", async () => {
    mockListDocuments.mockRejectedValue(new Error("Boom"));

    const { result } = renderHook(() => useDocumentsByIds(["d-1", "d-2"]), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.error).not.toBeNull();
    });

    expect(result.current.error).toBeInstanceOf(Error);
    expect(result.current.data).toEqual([]);
  });

  it("stabilizes query keys via sorting (same query for different order)", async () => {
    const doc1 = makeDocument("d-a");
    const doc2 = makeDocument("d-b");

    mockListDocuments.mockResolvedValue({ documents: [doc1, doc2] });

    const wrapper = makeWrapper();

    // Render with ["d-b", "d-a"] — hook should sort to ["d-a", "d-b"]
    const { result, rerender } = renderHook(
      ({ ids }: { ids: string[] }) => useDocumentsByIds(ids),
      { wrapper, initialProps: { ids: ["d-b", "d-a"] } },
    );

    await waitFor(() => {
      expect(result.current.data).toHaveLength(2);
    });

    const callCount = mockListDocuments.mock.calls.length;

    // Re-render with ["d-a", "d-b"] — same sorted key, no new query.
    rerender({ ids: ["d-a", "d-b"] });

    // Wait a tick for any potential refetches
    await new Promise((r) => setTimeout(r, 50));

    expect(mockListDocuments.mock.calls.length).toBe(callCount);
  });

  it("refetches when the id set changes", async () => {
    mockListDocuments.mockImplementation(
      ({ ids }: { ids: string }) => {
        const idList = ids.split(",");
        return Promise.resolve({
          documents: idList.map((id) => makeDocument(id)),
        });
      },
    );

    const wrapper = makeWrapper();

    const { result, rerender } = renderHook(
      ({ ids }: { ids: string[] }) => useDocumentsByIds(ids),
      { wrapper, initialProps: { ids: ["d-1"] } },
    );

    await waitFor(() => {
      expect(result.current.data).toHaveLength(1);
    });

    expect(mockListDocuments).toHaveBeenCalledWith({ ids: "d-1", limit: 1 });

    rerender({ ids: ["d-1", "d-2"] });

    await waitFor(() => {
      expect(result.current.data).toHaveLength(2);
    });

    expect(mockListDocuments).toHaveBeenCalledWith({
      ids: "d-1,d-2",
      limit: 2,
    });
  });
});
