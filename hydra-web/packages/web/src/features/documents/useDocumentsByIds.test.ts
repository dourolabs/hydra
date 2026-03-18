import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type { DocumentVersionRecord } from "@hydra/api";

// --- Mocks ---

const mockGetDocument = vi.fn();

vi.mock("../../api/client", () => ({
  apiClient: {
    getDocument: (...args: unknown[]) => mockGetDocument(...args),
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

function makeDocument(id: string): DocumentVersionRecord {
  return {
    document_id: id,
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    document: {
      title: `Document ${id}`,
      body_markdown: "",
    },
  } as DocumentVersionRecord;
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
    expect(mockGetDocument).not.toHaveBeenCalled();
  });

  it("fetches and returns all documents", async () => {
    const doc1 = makeDocument("d-1");
    const doc2 = makeDocument("d-2");

    mockGetDocument.mockImplementation((id: string) => {
      if (id === "d-1") return Promise.resolve(doc1);
      if (id === "d-2") return Promise.resolve(doc2);
      return Promise.reject(new Error("Unknown document"));
    });

    const { result } = renderHook(() => useDocumentsByIds(["d-1", "d-2"]), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.data).toHaveLength(2);
    });

    expect(mockGetDocument).toHaveBeenCalledWith("d-1");
    expect(mockGetDocument).toHaveBeenCalledWith("d-2");

    const ids = result.current.data.map((d) => d.document_id);
    expect(ids).toContain("d-1");
    expect(ids).toContain("d-2");
    expect(result.current.error).toBeNull();
  });

  it("sets error when a getDocument call fails", async () => {
    const doc1 = makeDocument("d-ok");

    mockGetDocument.mockImplementation((id: string) => {
      if (id === "d-ok") return Promise.resolve(doc1);
      return Promise.reject(new Error("Not found"));
    });

    const { result } = renderHook(() => useDocumentsByIds(["d-fail", "d-ok"]), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.error).not.toBeNull();
    });

    expect(result.current.error).toBeInstanceOf(Error);
    // The successful document should still be in data
    const ids = result.current.data.map((d) => d.document_id);
    expect(ids).toContain("d-ok");
  });

  it("stabilizes query keys via sorting (same queries for different order)", async () => {
    const doc1 = makeDocument("d-a");
    const doc2 = makeDocument("d-b");

    mockGetDocument.mockImplementation((id: string) => {
      if (id === "d-a") return Promise.resolve(doc1);
      if (id === "d-b") return Promise.resolve(doc2);
      return Promise.reject(new Error("Unknown"));
    });

    const wrapper = makeWrapper();

    // Render with ["d-b", "d-a"] — hook should sort to ["d-a", "d-b"]
    const { result, rerender } = renderHook(
      ({ ids }: { ids: string[] }) => useDocumentsByIds(ids),
      { wrapper, initialProps: { ids: ["d-b", "d-a"] } },
    );

    await waitFor(() => {
      expect(result.current.data).toHaveLength(2);
    });

    const callCount = mockGetDocument.mock.calls.length;

    // Re-render with ["d-a", "d-b"] — same sorted key, no new queries
    rerender({ ids: ["d-a", "d-b"] });

    // Wait a tick for any potential refetches
    await new Promise((r) => setTimeout(r, 50));

    // No additional getDocument calls should have been made
    expect(mockGetDocument.mock.calls.length).toBe(callCount);
  });
});
