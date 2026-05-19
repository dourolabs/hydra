// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type { ListDocumentsResponse, SearchDocumentsQuery } from "@hydra/api";

const mockListDocuments = vi.fn();

vi.mock("../../api/client", () => ({
  apiClient: {
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

function response(totalCount: bigint | null = null): ListDocumentsResponse {
  return {
    documents: [],
    next_cursor: null,
    total_count: totalCount,
  };
}

const { useDocumentCount } = await import("./useDocumentCount");

describe("useDocumentCount", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("requests count=true and limit=0, returning total_count as a number", async () => {
    mockListDocuments.mockResolvedValueOnce(response(247n));

    const { result } = renderHook(() => useDocumentCount(), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.data).toBe(247);
    });

    const call = mockListDocuments.mock.calls[0][0] as Partial<SearchDocumentsQuery>;
    expect(call.count).toBe(true);
    expect(call.limit).toBe(0);
  });

  it("returns 0 when total_count is missing/null", async () => {
    mockListDocuments.mockResolvedValueOnce(response(null));

    const { result } = renderHook(() => useDocumentCount(), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.data).toBe(0);
    });
  });

  it("respects the enabled flag", async () => {
    mockListDocuments.mockResolvedValue(response(5n));

    const { result } = renderHook(() => useDocumentCount(false), {
      wrapper: makeWrapper(),
    });

    // Give react-query a tick to run if it were going to.
    await new Promise((resolve) => setTimeout(resolve, 10));

    expect(mockListDocuments).not.toHaveBeenCalled();
    expect(result.current.data).toBeUndefined();
  });
});
