// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor, act } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type {
  IssueSummaryRecord,
  ListIssuesResponse,
  SearchIssuesQuery,
  StatusDefinition,
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

function makeStatus(key: string, label = key): StatusDefinition {
  return {
    key,
    label,
    color: "#3498db",
    position: 0,
    unblocks_parents: false,
    unblocks_dependents: false,
    cascades_to_children: false,
  };
}

const DEFAULT_STATUSES: StatusDefinition[] = [
  makeStatus("open"),
  makeStatus("in-progress"),
  makeStatus("failed"),
  makeStatus("closed"),
  makeStatus("dropped"),
];

function issue(
  id: string,
  status: string,
  projectId: string | null = null,
): IssueSummaryRecord {
  return {
    issue_id: id,
    version: 1n,
    timestamp: "2026-05-01T00:00:00.000Z",
    issue: {
      type: "task",
      title: id,
      description: "",
      creator: "alice",
      status: makeStatus(status),
      assignee: null,
      session_settings: null,
      dependencies: [],
      patches: [],
      project_id: projectId,
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

const { useBoardIssuesByProject } = await import("../usePaginatedIssues");

const DEFAULT_PROJECT = {
  project_id: "j-defaul",
  key: "default",
  name: "Default",
  statuses: DEFAULT_STATUSES,
};

describe("useBoardIssuesByProject", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("fires one bucketed request and groups results into per-cell maps", async () => {
    mockListIssues.mockImplementation(() =>
      Promise.resolve(
        page(
          DEFAULT_STATUSES.map((s) => issue(`i-${s.key}`, s.key, "j-defaul")),
        ),
      ),
    );

    const { result } = renderHook(
      () => useBoardIssuesByProject({}, [DEFAULT_PROJECT]),
      { wrapper: makeWrapper() },
    );

    await waitFor(() => {
      for (const s of DEFAULT_STATUSES) {
        expect(
          result.current.get("j-defaul")!.get(s.key)!.issues.length,
        ).toBe(1);
      }
    });

    // The fan-out (one request per cell) is gone — a single bucketed call
    // returns top-N issues per (project, status) cell in one roundtrip.
    expect(mockListIssues).toHaveBeenCalledTimes(1);
    const arg = mockListIssues.mock.calls[0][0] as Partial<SearchIssuesQuery>;
    expect(arg.bucket_by).toBe("project_status");
    expect(arg.bucket_limit).toBe(7);
    expect(arg.sort).toBe("project_status_time_desc");
    expect(arg.cursor).toBeUndefined();
    // No per-cell project/status filter on the bulk call.
    expect(arg.project_id).toBeUndefined();
    expect(arg.status).toBeUndefined();
    // No global `limit` on the bulk call — the backend applies `limit` as a
    // global cap *after* per-bucket truncation, so a default 50 would
    // silently empty later projects' cells in a populated workspace.
    expect(arg.limit).toBeUndefined();
  });

  it("groups bulk response across multiple projects into the correct cells", async () => {
    const projectAlpha = {
      project_id: "j-alpha",
      key: "alpha",
      name: "Alpha",
      statuses: [makeStatus("inbox"), makeStatus("done")],
    };
    const projectBeta = {
      project_id: "j-beta",
      key: "beta",
      name: "Beta",
      statuses: [makeStatus("backlog"), makeStatus("active"), makeStatus("shipped")],
    };

    mockListIssues.mockImplementation(() =>
      Promise.resolve(
        page([
          issue("a-inbox", "inbox", "j-alpha"),
          issue("a-done", "done", "j-alpha"),
          issue("b-backlog", "backlog", "j-beta"),
          issue("b-active", "active", "j-beta"),
          issue("b-shipped", "shipped", "j-beta"),
        ]),
      ),
    );

    const { result } = renderHook(
      () => useBoardIssuesByProject({}, [projectAlpha, projectBeta]),
      { wrapper: makeWrapper() },
    );

    await waitFor(() => {
      expect(result.current.get("j-alpha")!.get("inbox")!.issues.length).toBe(1);
      expect(result.current.get("j-beta")!.get("shipped")!.issues.length).toBe(1);
    });

    // One bulk call total — no per-project / per-status fan-out.
    expect(mockListIssues).toHaveBeenCalledTimes(1);
    expect(
      result.current.get("j-alpha")!.get("done")!.issues[0].issue_id,
    ).toBe("a-done");
    expect(
      result.current.get("j-beta")!.get("backlog")!.issues[0].issue_id,
    ).toBe("b-backlog");
    expect(
      result.current.get("j-beta")!.get("active")!.issues[0].issue_id,
    ).toBe("b-active");
  });

  it("fetchNextPage on a cell spawns a single-cell unbucketed cursor query", async () => {
    mockListIssues.mockImplementation((query: Partial<SearchIssuesQuery>) => {
      // Bulk bucketed request: one record per status, each at the
      // BOARD_PAGE_SIZE=7 boundary so the "open" cell reports hasNextPage.
      if (query.bucket_by === "project_status") {
        const issues: IssueSummaryRecord[] = [];
        for (const s of DEFAULT_STATUSES) {
          // Seven records in the "open" cell so the heuristic kicks in.
          const n = s.key === "open" ? 7 : 1;
          for (let i = 0; i < n; i++) {
            issues.push(issue(`i-${s.key}-${i}`, s.key, "j-defaul"));
          }
        }
        return Promise.resolve(page(issues));
      }
      // Per-cell unbucketed request after a Load more click.
      if (query.status === "open" && query.cursor === "open-next") {
        return Promise.resolve(page([issue("open-page2", "open", "j-defaul")], null));
      }
      if (query.status === "open") {
        return Promise.resolve(
          page([issue("open-page1", "open", "j-defaul")], "open-next"),
        );
      }
      return Promise.resolve(page([], null));
    });

    const { result } = renderHook(
      () => useBoardIssuesByProject({}, [DEFAULT_PROJECT]),
      { wrapper: makeWrapper() },
    );

    await waitFor(() => {
      expect(result.current.get("j-defaul")!.get("open")!.hasNextPage).toBe(
        true,
      );
    });

    const before = mockListIssues.mock.calls.length;
    expect(before).toBe(1); // bulk only

    await act(async () => {
      result.current.get("j-defaul")!.get("open")!.fetchNextPage();
    });

    await waitFor(() => {
      // The expanded query walks 2 pages on first click (depth=2): page-1
      // and page-2 of the single-cell cursor chain. Other cells are not
      // refetched — only the expanded cell makes follow-up requests.
      expect(
        result.current.get("j-defaul")!.get("open")!.issues.length,
      ).toBeGreaterThanOrEqual(2);
    });

    const followUpCalls = mockListIssues.mock.calls.slice(before);
    expect(followUpCalls.length).toBeGreaterThanOrEqual(1);
    for (const call of followUpCalls) {
      const arg = call[0] as Partial<SearchIssuesQuery>;
      // Follow-ups are single-cell unbucketed cursor queries scoped to
      // the expanded (project, status) cell.
      expect(arg.bucket_by).toBeUndefined();
      expect(arg.bucket_limit).toBeUndefined();
      expect(arg.project_id).toBe("j-defaul");
      expect(arg.status).toBe("open");
      expect(arg.limit).toBe(7);
    }
    const cursors = followUpCalls.map(
      (c) => (c[0] as Partial<SearchIssuesQuery>).cursor,
    );
    expect(cursors).toContain("open-next");
  });

  it("includes base filters (q, labels, creator, assignee) on the bulk query", async () => {
    mockListIssues.mockImplementation(() => Promise.resolve(page([])));

    renderHook(
      () =>
        useBoardIssuesByProject(
          {
            q: "needle",
            labels: "lbl-1",
            creator: "alice",
            assignee: "bob",
          },
          [DEFAULT_PROJECT],
        ),
      { wrapper: makeWrapper() },
    );

    await waitFor(() => {
      expect(mockListIssues).toHaveBeenCalledTimes(1);
    });

    const arg = mockListIssues.mock.calls[0][0] as Partial<SearchIssuesQuery>;
    expect(arg.q).toBe("needle");
    expect(arg.labels).toBe("lbl-1");
    expect(arg.creator).toBe("alice");
    expect(arg.assignee).toBe("bob");
    expect(arg.bucket_by).toBe("project_status");
    expect(arg.bucket_limit).toBe(7);
  });

  it("with chip status set, bucketing narrows the response and other columns are empty", async () => {
    mockListIssues.mockImplementation(() =>
      Promise.resolve(
        page([
          issue("a-open", "open", "j-defaul"),
          issue("b-open", "open", "j-defaul"),
        ]),
      ),
    );

    const { result } = renderHook(
      () =>
        useBoardIssuesByProject({ status: "open" }, [DEFAULT_PROJECT]),
      { wrapper: makeWrapper() },
    );

    await waitFor(() => {
      expect(result.current.get("j-defaul")!.get("open")!.issues.length).toBe(2);
    });

    // One bulk request, status filter and bucketing both applied.
    expect(mockListIssues).toHaveBeenCalledTimes(1);
    const arg = mockListIssues.mock.calls[0][0] as Partial<SearchIssuesQuery>;
    expect(arg.status).toBe("open");
    expect(arg.bucket_by).toBe("project_status");

    // Non-matching columns render zero rows and have no Load more.
    for (const s of DEFAULT_STATUSES) {
      if (s.key === "open") continue;
      const cell = result.current.get("j-defaul")!.get(s.key)!;
      expect(cell.issues.length).toBe(0);
      expect(cell.hasNextPage).toBe(false);
    }
  });
});
