// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type {
  IssueSummaryRecord,
  ListIssuesResponse,
  SearchIssuesQuery,
} from "@hydra/api";

const mockListIssues = vi.fn();

vi.mock("../../../api/client", () => ({
  apiClient: {
    listIssues: (...args: unknown[]) => mockListIssues(...args),
  },
}));

function makeWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return ({ children }: { children: React.ReactNode }) =>
    React.createElement(QueryClientProvider, { client: queryClient }, children);
}

function page(
  issues: IssueSummaryRecord[] = [],
  nextCursor: string | null = null,
): ListIssuesResponse {
  return { issues, next_cursor: nextCursor } as ListIssuesResponse;
}

const { usePaginatedIssues, useIssueCount } = await import(
  "../usePaginatedIssues"
);

describe("usePaginatedIssues sort", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  // PR-2: the list page asks the server to group by project then status so
  // the renderer can iterate the stream without re-sorting. This sort is
  // only on the table-view caller — board (`useBoardIssuesByProject`) and
  // the count query stay on the default.
  it("passes sort=project_status_time_desc to listIssues", async () => {
    mockListIssues.mockResolvedValue(page());

    renderHook(() => usePaginatedIssues({}), { wrapper: makeWrapper() });

    await waitFor(() => {
      expect(mockListIssues).toHaveBeenCalled();
    });
    const arg = mockListIssues.mock.calls[0][0] as Partial<SearchIssuesQuery>;
    expect(arg.sort).toBe("project_status_time_desc");
  });

  it("threads filter values through alongside the sort", async () => {
    mockListIssues.mockResolvedValue(page());

    renderHook(
      () => usePaginatedIssues({ status: "in-progress", project_id: "j-eng" }),
      { wrapper: makeWrapper() },
    );

    await waitFor(() => {
      expect(mockListIssues).toHaveBeenCalled();
    });
    const arg = mockListIssues.mock.calls[0][0] as Partial<SearchIssuesQuery>;
    expect(arg.status).toBe("in-progress");
    expect(arg.project_id).toBe("j-eng");
    expect(arg.sort).toBe("project_status_time_desc");
  });

  // PR-4 will move the count query onto the new sort if needed; for now it
  // keeps the historical shape, so callers that key off the count payload
  // aren't perturbed.
  it("does NOT pass sort on useIssueCount", async () => {
    mockListIssues.mockResolvedValue({
      issues: [],
      next_cursor: null,
      total_count: 0n,
    } as unknown as ListIssuesResponse);

    renderHook(() => useIssueCount({}), { wrapper: makeWrapper() });

    await waitFor(() => {
      expect(mockListIssues).toHaveBeenCalled();
    });
    const arg = mockListIssues.mock.calls[0][0] as Partial<SearchIssuesQuery>;
    expect(arg.sort).toBeUndefined();
  });
});
