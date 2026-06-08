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
      progress: "",
      status,
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
  default_status_key: "open",
};

describe("useBoardIssuesByProject", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("fires one paginated query per (project, status) cell with limit=50", async () => {
    mockListIssues.mockImplementation((query: Partial<SearchIssuesQuery>) =>
      Promise.resolve(page([issue(`i-${query.status}`, query.status as string)])),
    );

    const { result } = renderHook(
      () => useBoardIssuesByProject({}, [DEFAULT_PROJECT]),
      { wrapper: makeWrapper() },
    );

    await waitFor(() => {
      for (const s of DEFAULT_STATUSES) {
        expect(result.current.get("j-defaul")!.get(s.key)!.issues.length).toBe(1);
      }
    });

    expect(mockListIssues).toHaveBeenCalledTimes(DEFAULT_STATUSES.length);
    const statuses = mockListIssues.mock.calls.map(
      (c) => (c[0] as Partial<SearchIssuesQuery>).status,
    );
    for (const s of DEFAULT_STATUSES) {
      expect(statuses).toContain(s.key);
    }
    for (const call of mockListIssues.mock.calls) {
      const arg = call[0] as Partial<SearchIssuesQuery>;
      expect(arg.limit).toBe(50);
      expect(arg.cursor).toBeUndefined();
    }
  });

  it("dispatches per-(project, status) cells across multiple projects", async () => {
    const projectAlpha = {
      project_id: "j-alpha",
      key: "alpha",
      name: "Alpha",
      statuses: [makeStatus("inbox"), makeStatus("done")],
      default_status_key: "inbox",
    };
    const projectBeta = {
      project_id: "j-beta",
      key: "beta",
      name: "Beta",
      statuses: [makeStatus("backlog"), makeStatus("active"), makeStatus("shipped")],
      default_status_key: "backlog",
    };

    mockListIssues.mockImplementation((query: Partial<SearchIssuesQuery>) =>
      Promise.resolve(
        page([
          issue(
            `i-${query.project_id ?? "none"}-${query.status}`,
            query.status as string,
            (query.project_id as string) ?? null,
          ),
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

    // 2 (alpha) + 3 (beta) cells = 5 server requests.
    expect(mockListIssues).toHaveBeenCalledTimes(5);
    const seen = new Set<string>();
    for (const call of mockListIssues.mock.calls) {
      const arg = call[0] as Partial<SearchIssuesQuery>;
      seen.add(`${arg.project_id ?? "_"}::${arg.status}`);
    }
    expect(seen.has("j-alpha::inbox")).toBe(true);
    expect(seen.has("j-alpha::done")).toBe(true);
    expect(seen.has("j-beta::backlog")).toBe(true);
    expect(seen.has("j-beta::active")).toBe(true);
    expect(seen.has("j-beta::shipped")).toBe(true);
  });

  it("fetchNextPage on a column passes that column's cursor and status only", async () => {
    mockListIssues.mockImplementation((query: Partial<SearchIssuesQuery>) => {
      if (query.status === "open" && query.cursor === "open-next") {
        return Promise.resolve(page([issue("open-2", "open")], null));
      }
      if (query.status === "open" && !query.cursor) {
        return Promise.resolve(page([issue("open-1", "open")], "open-next"));
      }
      return Promise.resolve(
        page([issue(`i-${query.status}`, query.status as string)], null),
      );
    });

    const { result } = renderHook(
      () => useBoardIssuesByProject({}, [DEFAULT_PROJECT]),
      { wrapper: makeWrapper() },
    );

    await waitFor(() => {
      expect(result.current.get("j-defaul")!.get("open")!.hasNextPage).toBe(true);
      expect(result.current.get("j-defaul")!.get("in-progress")!.issues.length).toBe(1);
    });

    const before = mockListIssues.mock.calls.length;

    await act(async () => {
      result.current.get("j-defaul")!.get("open")!.fetchNextPage();
    });

    await waitFor(() => {
      expect(result.current.get("j-defaul")!.get("open")!.issues.length).toBe(2);
    });

    // After the depth bump, the open cell refetches its full chain
    // (page-1 + page-2). Other cells are unaffected.
    const after = mockListIssues.mock.calls.length;
    const followUpCalls = mockListIssues.mock.calls.slice(before);
    const followUpOpenCursors = followUpCalls
      .filter((c) => (c[0] as Partial<SearchIssuesQuery>).status === "open")
      .map((c) => (c[0] as Partial<SearchIssuesQuery>).cursor);
    expect(followUpOpenCursors).toContain("open-next");
    expect(after - before).toBeGreaterThanOrEqual(1);
  });

  it("includes base filters (q, labels, creator, assignee) in every cell query", async () => {
    mockListIssues.mockImplementation((query: Partial<SearchIssuesQuery>) =>
      Promise.resolve(page([issue(`i-${query.status}`, query.status as string)])),
    );

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
      expect(mockListIssues).toHaveBeenCalledTimes(DEFAULT_STATUSES.length);
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
    mockListIssues.mockImplementation((query: Partial<SearchIssuesQuery>) =>
      Promise.resolve(
        page([
          issue(`a-${query.status}`, query.status as string),
          issue(`b-${query.status}`, query.status as string),
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

    // Every actual network call must use the chip status — the cell
    // queries within a project share a cache key (status=open) so
    // React Query dedupes to 1 network call per project.
    for (const call of mockListIssues.mock.calls) {
      const arg = call[0] as Partial<SearchIssuesQuery>;
      expect(arg.status).toBe("open");
    }

    // Non-matching columns render zero rows and have no Load more.
    for (const s of DEFAULT_STATUSES) {
      if (s.key === "open") continue;
      const cell = result.current.get("j-defaul")!.get(s.key)!;
      expect(cell.issues.length).toBe(0);
      expect(cell.hasNextPage).toBe(false);
    }
  });
});
