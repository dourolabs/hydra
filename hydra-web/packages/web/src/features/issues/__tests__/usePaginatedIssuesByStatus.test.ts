// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor, act } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type {
  IssueStatus,
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

function issue(id: string, status: IssueStatus): IssueSummaryRecord {
  return {
    issue_id: id,
    version: 1n,
    timestamp: "2026-05-01T00:00:00.000Z",
    issue: {
      type: "task",
      title: id,
      description: "",
      creator: "alice",
      progress: "",
      status,
      assignee: null,
      session_settings: null,
      dependencies: [],
      patches: [],
    },
    creation_time: "2026-05-01T00:00:00.000Z",
  } as unknown as IssueSummaryRecord;
}

function page(
  issues: IssueSummaryRecord[],
  nextCursor: string | null = null,
): ListIssuesResponse {
  return { issues, next_cursor: nextCursor } as ListIssuesResponse;
}

const { usePaginatedIssuesByStatus, BOARD_STATUSES } = await import(
  "../usePaginatedIssues"
);

describe("usePaginatedIssuesByStatus", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("fires one paginated query per column status with limit=50", async () => {
    mockListIssues.mockImplementation((query: Partial<SearchIssuesQuery>) =>
      Promise.resolve(page([issue(`i-${query.status}`, query.status as IssueStatus)])),
    );

    const { result } = renderHook(() => usePaginatedIssuesByStatus({}), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      // All five columns finish their initial fetch.
      for (const s of BOARD_STATUSES) {
        expect(result.current[s].issues.length).toBe(1);
      }
    });

    expect(mockListIssues).toHaveBeenCalledTimes(5);
    const statuses = mockListIssues.mock.calls.map(
      (c) => (c[0] as Partial<SearchIssuesQuery>).status,
    );
    for (const s of BOARD_STATUSES) {
      expect(statuses).toContain(s);
    }
    // limit=50 and no cursor for the initial page of every column.
    for (const call of mockListIssues.mock.calls) {
      const arg = call[0] as Partial<SearchIssuesQuery>;
      expect(arg.limit).toBe(50);
      expect(arg.cursor).toBeUndefined();
    }
  });

  it("fetchNextPage on a column passes that column's cursor and status only", async () => {
    mockListIssues.mockImplementation((query: Partial<SearchIssuesQuery>) => {
      if (query.status === "open" && query.cursor === "open-next") {
        return Promise.resolve(page([issue("open-2", "open")], null));
      }
      if (query.status === "open") {
        return Promise.resolve(page([issue("open-1", "open")], "open-next"));
      }
      // Other columns return a single page, no next.
      return Promise.resolve(
        page([issue(`i-${query.status}`, query.status as IssueStatus)], null),
      );
    });

    const { result } = renderHook(() => usePaginatedIssuesByStatus({}), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.open.hasNextPage).toBe(true);
      expect(result.current["in-progress"].issues.length).toBe(1);
    });

    expect(mockListIssues).toHaveBeenCalledTimes(5);

    await act(async () => {
      result.current.open.fetchNextPage();
    });

    await waitFor(() => {
      expect(result.current.open.issues.length).toBe(2);
    });

    expect(mockListIssues).toHaveBeenCalledTimes(6);
    const followUp = mockListIssues.mock.calls[5][0] as Partial<SearchIssuesQuery>;
    expect(followUp.status).toBe("open");
    expect(followUp.cursor).toBe("open-next");
    expect(followUp.limit).toBe(50);
  });

  it("includes base filters (q, labels, creator, assignee) in every column query", async () => {
    mockListIssues.mockImplementation((query: Partial<SearchIssuesQuery>) =>
      Promise.resolve(page([issue(`i-${query.status}`, query.status as IssueStatus)])),
    );

    renderHook(
      () =>
        usePaginatedIssuesByStatus({
          q: "needle",
          labels: "lbl-1",
          creator: "alice",
          assignee: "bob",
        }),
      { wrapper: makeWrapper() },
    );

    await waitFor(() => {
      expect(mockListIssues).toHaveBeenCalledTimes(5);
    });

    for (const call of mockListIssues.mock.calls) {
      const arg = call[0] as Partial<SearchIssuesQuery>;
      expect(arg.q).toBe("needle");
      expect(arg.labels).toBe("lbl-1");
      expect(arg.creator).toBe("alice");
      expect(arg.assignee).toBe("bob");
    }
  });

  it("when the chip status is set, only the matching column shows issues", async () => {
    mockListIssues.mockImplementation((query: Partial<SearchIssuesQuery>) => {
      // Server returns issues matching the requested status.
      return Promise.resolve(
        page([
          issue(`a-${query.status}`, query.status as IssueStatus),
          issue(`b-${query.status}`, query.status as IssueStatus),
        ]),
      );
    });

    const { result } = renderHook(
      () => usePaginatedIssuesByStatus({ status: "open" }),
      { wrapper: makeWrapper() },
    );

    await waitFor(() => {
      expect(result.current.open.issues.length).toBe(2);
    });

    // Every actual network call must use the chip status — the 5 column
    // queries share a cache key (status=open) so React Query dedupes to 1
    // network call.
    for (const call of mockListIssues.mock.calls) {
      const arg = call[0] as Partial<SearchIssuesQuery>;
      expect(arg.status).toBe("open");
    }

    // Non-matching columns render zero rows and have no Load more.
    for (const s of BOARD_STATUSES) {
      if (s === "open") continue;
      expect(result.current[s].issues.length).toBe(0);
      expect(result.current[s].hasNextPage).toBe(false);
    }
  });
});
