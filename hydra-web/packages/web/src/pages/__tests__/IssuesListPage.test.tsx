// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, cleanup } from "@testing-library/react";
import { MemoryRouter, Routes, Route, useLocation } from "react-router-dom";
import type { ChildStatus } from "../../features/dashboard/computeIssueProgress";
import type { IssueSummaryRecord, SessionSummaryRecord } from "@hydra/api";

// --- Mocks ---

// IssuesView replaced the four Pickers with the generic <FilterBar>. The
// FilterBar drives client-side filtering — server fetches in table mode now
// load the unfiltered page. The test stubs FilterBar to a no-op div so this
// file stays focused on URL → page-framing wiring (eyebrow, title,
// breadcrumb) and the server-fetch surface for the table layout.
vi.mock("../../features/filters", () => ({
  FilterBar: () => <div data-testid="filter-bar" />,
  applyFilters: <T,>(items: T[]) => items,
}));

vi.mock("../../features/issues/issueFilters", () => ({
  useIssueFilters: () => ({}),
}));

// Relation resolver issues `/v1/relations` queries via useQueries, which
// needs a QueryClient. Stub it to a no-op `null` result (no relation filter
// active) so the page exercises the URL → server-query mapping for the
// scalar filters covered by the rest of this file. The `relationsState`
// object lets the few relation-aware tests below flip the resolver into
// a loading state to verify the "previous render persists" behaviour.
const relationsState: {
  issueIds: string[] | null;
  isLoading: boolean;
} = { issueIds: null, isLoading: false };

vi.mock("../../features/issues/useRelationFilteredIssueIds", () => ({
  useRelationFilteredIssueIds: () => ({
    issueIds: relationsState.issueIds,
    isLoading: relationsState.isLoading,
  }),
  RELATION_FILTER_IDS: [
    "relatedPatch",
    "relatedChat",
    "relatedSession",
    "parentOrChild",
  ],
}));

interface PaginatedState {
  issues: IssueSummaryRecord[];
  totalCount: number | undefined;
  paginatedFilters: unknown;
  countFilters: unknown;
}
const paginatedState: PaginatedState = {
  issues: [],
  totalCount: undefined,
  paginatedFilters: undefined,
  countFilters: undefined,
};

vi.mock("../../features/issues/usePaginatedIssues", () => ({
  usePaginatedIssues: (filters: unknown) => {
    paginatedState.paginatedFilters = filters;
    return {
      data: { pages: [{ issues: paginatedState.issues }] },
      isLoading: false,
      fetchNextPage: vi.fn(),
      hasNextPage: false,
      isFetchingNextPage: false,
    };
  },
  useIssueCount: (filters: unknown) => {
    paginatedState.countFilters = filters;
    return { data: paginatedState.totalCount };
  },
  // Board owns its own fetches via this hook. The test suite renders the
  // default (table) layout, but the import graph still pulls IssuesBoard in
  // so the mock must export every name IssuesBoard reads.
  usePaginatedIssuesByStatus: () => ({
    open: {
      issues: [],
      isLoading: false,
      hasNextPage: false,
      isFetchingNextPage: false,
      fetchNextPage: vi.fn(),
    },
    "in-progress": {
      issues: [],
      isLoading: false,
      hasNextPage: false,
      isFetchingNextPage: false,
      fetchNextPage: vi.fn(),
    },
    failed: {
      issues: [],
      isLoading: false,
      hasNextPage: false,
      isFetchingNextPage: false,
      fetchNextPage: vi.fn(),
    },
    closed: {
      issues: [],
      isLoading: false,
      hasNextPage: false,
      isFetchingNextPage: false,
      fetchNextPage: vi.fn(),
    },
    dropped: {
      issues: [],
      isLoading: false,
      hasNextPage: false,
      isFetchingNextPage: false,
      fetchNextPage: vi.fn(),
    },
  }),
  BOARD_STATUSES: ["open", "in-progress", "failed", "closed", "dropped"],
}));

vi.mock("../../features/auth/useAuth", () => ({
  useAuth: () => ({
    user: { actor: { type: "user", username: "alice" } },
    loading: false,
    logout: vi.fn(),
  }),
}));

vi.mock("../../api/auth", () => ({
  actorDisplayName: () => "alice",
  // The mocked user is `{ type: "user", username: "alice" }`, so the
  // Phase 4b Principal path form is `users/alice`.
  actorPrincipalPath: () => "users/alice",
}));

interface TreesState {
  childStatusMap: Map<string, ChildStatus[]>;
  sessionsByIssue: Map<string, SessionSummaryRecord[]>;
}
const treesState: TreesState = {
  childStatusMap: new Map(),
  sessionsByIssue: new Map(),
};

vi.mock("../../features/dashboard/usePageIssueTrees", () => ({
  usePageIssueTrees: () => ({
    isActiveMap: new Map(),
    childStatusMap: treesState.childStatusMap,
    sessionsByIssue: treesState.sessionsByIssue,
    isLoading: false,
  }),
}));

vi.mock("../../features/projects/useProjects", () => ({
  useProjects: () => ({ data: [] }),
  useProject: () => ({ data: null }),
  useProjectStatuses: () => ({ data: null }),
}));

const useBreadcrumbsMock = vi.fn();
vi.mock("../../layout/useBreadcrumbs", () => ({
  useBreadcrumbs: (...args: unknown[]) => useBreadcrumbsMock(...args),
}));

const openIssueCreateModalMock = vi.fn();
vi.mock("../../features/dashboard/useIssueCreateModal", () => ({
  useIssueCreateModal: () => ({
    isOpen: false,
    open: openIssueCreateModalMock,
    close: vi.fn(),
  }),
}));

vi.mock("../IssuesListPage.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("../../features/issues/view/IssuesView.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("../../features/issues/view/IssuesTable.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("@hydra/ui", () => ({
  Avatar: ({ name, kind }: { name: string; kind?: string }) => (
    <span data-testid="avatar" data-kind={kind ?? "human"}>
      {name}
    </span>
  ),
  Badge: ({ status }: { status: string }) => <span data-testid="badge">{status}</span>,
  TypeChip: ({ type }: { type: string }) => <span data-testid="type-chip">{type}</span>,
  Icons: new Proxy(
    {},
    {
      get: () => () => <span data-testid="icon" />,
    },
  ),
}));

// --- Import after mocks ---
const { IssuesListPage } = await import("../IssuesListPage");

function LocationDisplay() {
  const location = useLocation();
  return (
    <div data-testid="location">
      {location.pathname}
      {location.search}
    </div>
  );
}

function renderIssuesList(initialEntry: string) {
  return render(
    <MemoryRouter initialEntries={[initialEntry]}>
      <Routes>
        <Route path="/" element={<IssuesListPage />} />
      </Routes>
      <LocationDisplay />
    </MemoryRouter>,
  );
}

function makeIssue(
  id: string,
  overrides: Partial<IssueSummaryRecord["issue"]> = {},
): IssueSummaryRecord {
  return {
    issue_id: id,
    version: 1n,
    timestamp: "2026-03-15T10:00:00.000Z",
    issue: {
      type: "task",
      title: `Issue ${id}`,
      description: "",
      creator: "alice",
      progress: "",
      status: "open",
      assignee: null,
      session_settings: null,
      dependencies: [],
      patches: [],
      ...overrides,
    },
    creation_time: "2026-03-15T10:00:00.000Z",
  } as unknown as IssueSummaryRecord;
}

function makeSession(
  id: string,
  issueId: string,
  status: SessionSummaryRecord["session"]["status"],
  opts: { startTime?: string | null; endTime?: string | null } = {},
): SessionSummaryRecord {
  return {
    session_id: id,
    version: 1n,
    timestamp: "2026-03-15T10:00:00.000Z",
    session: {
      prompt: "",
      creator: "swe",
      status,
      spawned_from: issueId,
      start_time: opts.startTime ?? null,
      end_time: opts.endTime ?? null,
    },
  } as unknown as SessionSummaryRecord;
}

beforeEach(() => {
  vi.clearAllMocks();
  paginatedState.issues = [];
  paginatedState.totalCount = undefined;
  paginatedState.paginatedFilters = undefined;
  paginatedState.countFilters = undefined;
  treesState.childStatusMap = new Map();
  treesState.sessionsByIssue = new Map();
  relationsState.issueIds = null;
  relationsState.isLoading = false;
});

afterEach(() => {
  cleanup();
  openIssueCreateModalMock.mockReset();
});

describe("IssuesListPage Issue Create modal", () => {
  // The "+ Create Issue" button moved from the dashboard body to the topbar in
  // the design refresh — its create flow is exercised in SiteHeader tests now.
  it("does not mount its own IssueCreateModal", () => {
    renderIssuesList("/?create-issue=1");
    expect(document.querySelector('[data-testid="issue-create-modal"]')).toBeNull();
  });
});

describe("IssuesListPage breadcrumb label", () => {
  it("publishes Workspace / All issues breadcrumb on the default view", () => {
    renderIssuesList("/");
    expect(useBreadcrumbsMock).toHaveBeenCalledWith(
      [{ label: "Workspace", to: "/" }],
      "All issues",
    );
  });

  it("publishes Workspace / My issues when ?creator=<user> is set", () => {
    renderIssuesList("/?creator=alice");
    expect(useBreadcrumbsMock).toHaveBeenCalledWith(
      [{ label: "Workspace", to: "/" }],
      "My issues",
    );
  });

  it("publishes Workspace / Assigned to me when ?selected=assigned", () => {
    renderIssuesList("/?selected=assigned");
    expect(useBreadcrumbsMock).toHaveBeenCalledWith(
      [{ label: "Workspace", to: "/" }],
      "Assigned to me",
    );
  });

  it("normalises legacy ?selected=patches back to the default All issues view", () => {
    renderIssuesList("/?selected=patches");
    // patches is no longer a dashboard tab — fall back to the default 'All issues'
    expect(useBreadcrumbsMock).toHaveBeenCalledWith(
      [{ label: "Workspace", to: "/" }],
      "All issues",
    );
  });
});

describe("IssuesListPage table-mode server filtering", () => {
  // The new FilterBar drives a server-side query: URL filter params translate
  // into `IssueFilters` on `usePaginatedIssues` / `useIssueCount`. The
  // URL-shortcut framing (`?creator=alice`, `?selected=your-issues`,
  // `?selected=in_progress`) also rides through to the server query.

  it("sends no filters to the server on a bare index route", () => {
    renderIssuesList("/");
    expect(paginatedState.paginatedFilters).toEqual({});
    expect(paginatedState.countFilters).toEqual({});
  });

  it("sends creator to the server when ?creator=<user> is set", () => {
    renderIssuesList("/?creator=alice");
    expect(paginatedState.paginatedFilters).toEqual({ creator: "alice" });
  });

  it("sends creator to the server for legacy ?selected=your-issues", () => {
    renderIssuesList("/?selected=your-issues");
    expect(paginatedState.paginatedFilters).toEqual({ creator: "alice" });
  });

  it("sends no filters to the server for ?selected=all", () => {
    renderIssuesList("/?selected=all");
    expect(paginatedState.paginatedFilters).toEqual({});
  });

  it("sends assignee to the server for legacy ?selected=assigned", () => {
    renderIssuesList("/?selected=assigned");
    expect(paginatedState.paginatedFilters).toEqual({
      assignee: "users/alice",
    });
  });

  it("sends status=in-progress for legacy ?selected=in_progress", () => {
    renderIssuesList("/?selected=in_progress");
    expect(paginatedState.paginatedFilters).toEqual({ status: "in-progress" });
  });

  it("sends type to the server when ?type=bug is set", () => {
    renderIssuesList("/?type=bug");
    expect(paginatedState.paginatedFilters).toEqual({ type: "bug" });
  });

  it("invokes the list and count hooks with identical filter inputs", () => {
    paginatedState.issues = [makeIssue("i-1")];
    paginatedState.totalCount = 9;

    renderIssuesList("/?selected=assigned");

    expect(paginatedState.paginatedFilters).toBeDefined();
    expect(paginatedState.countFilters).toBeDefined();
    expect(paginatedState.countFilters).toEqual(paginatedState.paginatedFilters);
  });

  it("sends the free-text ?q= search param to the server", () => {
    renderIssuesList("/?q=deployment");
    expect(paginatedState.paginatedFilters).toEqual({ q: "deployment" });
  });
});

describe("IssuesListPage eyebrow count", () => {
  it("renders total_count from the count query, not just the loaded page length", () => {
    paginatedState.issues = [makeIssue("i-1"), makeIssue("i-2")];
    paginatedState.totalCount = 247;

    const { container } = renderIssuesList("/?selected=all");

    const eyebrow = container.querySelector(".eyebrow");
    expect(eyebrow).not.toBeNull();
    expect(eyebrow!.textContent).toBe("ALL · 247 ISSUES");
  });

  it("falls back to the rendered issues length while the count query is loading", () => {
    paginatedState.issues = [makeIssue("i-1"), makeIssue("i-2"), makeIssue("i-3")];
    paginatedState.totalCount = undefined;

    const { container } = renderIssuesList("/");

    const eyebrow = container.querySelector(".eyebrow");
    expect(eyebrow).not.toBeNull();
    expect(eyebrow!.textContent).toBe("ALL · 3 ISSUES");
  });

  it("uses the singular form when the resolved count is exactly 1", () => {
    paginatedState.issues = [];
    paginatedState.totalCount = 1;

    const { container } = renderIssuesList("/?selected=in_progress");

    const eyebrow = container.querySelector(".eyebrow");
    expect(eyebrow!.textContent).toBe("IN PROGRESS · 1 ISSUE");
  });
});

describe("IssuesListPage IssuesTable rendering", () => {
  it("renders the Runtime cell with an active class when a session is running", () => {
    const issue = makeIssue("i-active", { title: "active row" });
    paginatedState.issues = [issue];
    treesState.sessionsByIssue = new Map([
      [
        issue.issue_id,
        [
          makeSession("s-1", issue.issue_id, "running", {
            startTime: new Date(Date.now() - 5_000).toISOString(),
          }),
        ],
      ],
    ]);

    const { getByTestId } = renderIssuesList("/");

    const cell = getByTestId("runtime-active");
    expect(cell).toBeDefined();
    expect(cell.className).toContain("isLive");
    // Elapsed >= 5 seconds — format starts with a digit and ends with "s".
    expect(cell.textContent).toMatch(/^\d+s$/);
  });

  it("renders the Runtime cell with an idle class for a completed-only session", () => {
    const issue = makeIssue("i-done", { title: "done row" });
    paginatedState.issues = [issue];
    treesState.sessionsByIssue = new Map([
      [
        issue.issue_id,
        [
          makeSession("s-1", issue.issue_id, "complete", {
            startTime: "2026-03-15T10:00:00.000Z",
            endTime: "2026-03-15T10:00:42.000Z",
          }),
        ],
      ],
    ]);

    const { getByTestId } = renderIssuesList("/");

    const cell = getByTestId("runtime-idle");
    expect(cell).toBeDefined();
    expect(cell.className).toContain("rtInstrument");
    expect(cell.className).not.toContain("isLive");
    expect(cell.textContent).toBe("42s");
  });

  it("applies the active glow class to the fill span when any child has hasActiveTask", () => {
    const issue = makeIssue("i-parent", { title: "parent row" });
    paginatedState.issues = [issue];
    treesState.childStatusMap = new Map([
      [
        issue.issue_id,
        [
          {
            id: "i-child-1",
            status: "in-progress",
            hasActiveTask: true,
            assignedToUser: false,
          },
          {
            id: "i-child-2",
            status: "closed",
            hasActiveTask: false,
            assignedToUser: false,
          },
        ],
      ],
    ]);

    const { container } = renderIssuesList("/");

    const progress = container.querySelector(".progress");
    expect(progress).not.toBeNull();
    // Container keeps progressActive so it can switch overflow to visible
    // and let the fill's outer shadow escape.
    expect(progress!.className).toContain("progressActive");

    const fill = progress!.querySelector(".progressFill");
    expect(fill).not.toBeNull();
    // The glow visual treatment now lives on the fill, not the container.
    expect(fill!.className).toContain("progressFillActive");
    // Projected fill = (closed + in-progress) / total = 2 / 2 = 100%.
    expect((fill as HTMLElement).style.width).toBe("100%");
  });

  it("sets the fill width to (closed + in-progress) / total", () => {
    const issue50 = makeIssue("i-half", { title: "half row" });
    const issue50b = makeIssue("i-half2", { title: "half row b" });
    paginatedState.issues = [issue50, issue50b];
    treesState.childStatusMap = new Map([
      [
        issue50.issue_id,
        [
          {
            id: "c1",
            status: "in-progress",
            hasActiveTask: false,
            assignedToUser: false,
          },
          {
            id: "c2",
            status: "open",
            hasActiveTask: false,
            assignedToUser: false,
          },
        ],
      ],
      [
        issue50b.issue_id,
        [
          {
            id: "c3",
            status: "closed",
            hasActiveTask: false,
            assignedToUser: false,
          },
          {
            id: "c4",
            status: "open",
            hasActiveTask: false,
            assignedToUser: false,
          },
        ],
      ],
    ]);

    const { container } = renderIssuesList("/");

    const fills = container.querySelectorAll(".progressFill");
    expect(fills.length).toBe(2);
    // [in-progress, open] → 1/2 = 50%
    expect((fills[0] as HTMLElement).style.width).toBe("50%");
    // [closed, open] → 1/2 = 50%
    expect((fills[1] as HTMLElement).style.width).toBe("50%");
    // No active child → fill should not carry the active class.
    expect(fills[0]!.className).not.toContain("progressFillActive");
    expect(fills[1]!.className).not.toContain("progressFillActive");
  });

  it("differentiates idle and active fills via the progressFillActive class", () => {
    const idleIssue = makeIssue("i-idle", { title: "idle row" });
    const activeIssue = makeIssue("i-active", { title: "active row" });
    paginatedState.issues = [idleIssue, activeIssue];
    treesState.childStatusMap = new Map([
      [
        idleIssue.issue_id,
        [
          {
            id: "i-idle-c1",
            status: "closed",
            hasActiveTask: false,
            assignedToUser: false,
          },
          {
            id: "i-idle-c2",
            status: "open",
            hasActiveTask: false,
            assignedToUser: false,
          },
        ],
      ],
      [
        activeIssue.issue_id,
        [
          {
            id: "i-active-c1",
            status: "in-progress",
            hasActiveTask: true,
            assignedToUser: false,
          },
          {
            id: "i-active-c2",
            status: "open",
            hasActiveTask: false,
            assignedToUser: false,
          },
        ],
      ],
    ]);

    const { container } = renderIssuesList("/");

    const fills = container.querySelectorAll(".progressFill");
    expect(fills.length).toBe(2);
    expect(fills[0]!.className).toContain("progressFill");
    expect(fills[0]!.className).not.toContain("progressFillActive");
    expect(fills[1]!.className).toContain("progressFill");
    expect(fills[1]!.className).toContain("progressFillActive");
  });
});

describe("IssuesListPage relation-resolution loading persistence", () => {
  // PR-2: When the relation resolver re-runs (user changed a relation chip
  // value), the page must keep rendering the previously-loaded rows from
  // the list query — driven by `placeholderData: keepPreviousData` on
  // `usePaginatedIssues` / `useIssueCount` — instead of flashing the
  // skeleton or zero-state. The fix drops `|| (isTable && relationsLoading)`
  // from the `isLoading` prop forwarded to <IssuesView>.

  it("keeps prior rows visible while relations are resolving", () => {
    paginatedState.issues = [
      makeIssue("i-prev-1", { title: "prior row 1" }),
      makeIssue("i-prev-2", { title: "prior row 2" }),
    ];
    relationsState.issueIds = ["i-prev-1", "i-prev-2"];
    relationsState.isLoading = true;

    const { container } = renderIssuesList("/?relatedPatch=p-a,p-b");

    // No loading or empty state — previous render persists.
    expect(container.textContent).not.toContain("Loading issues");
    expect(container.textContent).not.toContain(
      "No issues match the current filters",
    );
    // Both prior rows still in the DOM.
    expect(container.textContent).toContain("prior row 1");
    expect(container.textContent).toContain("prior row 2");
  });

  it("does not force the loading skeleton when relations re-resolve over an existing render", () => {
    paginatedState.issues = [makeIssue("i-keep", { title: "keep me" })];
    relationsState.issueIds = ["i-keep"];
    relationsState.isLoading = true;

    const { container } = renderIssuesList("/?relatedPatch=p-a");

    expect(container.textContent).not.toContain("Loading issues");
    expect(container.textContent).toContain("keep me");
  });
});
