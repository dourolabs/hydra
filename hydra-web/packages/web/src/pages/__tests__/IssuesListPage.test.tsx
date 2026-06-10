// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, cleanup } from "@testing-library/react";
import { MemoryRouter, Routes, Route, useLocation } from "react-router-dom";
import type { IssueSummaryRecord, SessionSummaryRecord } from "@hydra/api";
import type { IssueNeighborhood } from "../../features/issues/flowPill";

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
  useBoardIssuesByProject: () => new Map(),
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
  neighborhoodMap: Map<string, IssueNeighborhood>;
  sessionsByIssue: Map<string, SessionSummaryRecord[]>;
}
const treesState: TreesState = {
  neighborhoodMap: new Map(),
  sessionsByIssue: new Map(),
};

vi.mock("../../features/dashboard/usePageIssueTrees", () => ({
  usePageIssueTrees: () => ({
    neighborhoodMap: treesState.neighborhoodMap,
    sessionsByIssue: treesState.sessionsByIssue,
    isLoading: false,
  }),
}));

// Mutable holder so individual tests can stage a project list for the
// project-key resolver (see "IssuesListPage project key resolution" below).
// `data` is `undefined` while loading and an array (possibly empty) once
// loaded — matches the real `useProjects` query shape after `select:`.
interface ProjectsMockState {
  data: Array<{
    project_id: string;
    project: { key: string; name: string };
  }> | undefined;
}
const projectsState: ProjectsMockState = { data: [] };

vi.mock("../../features/projects/useProjects", () => ({
  useProjects: () => ({ data: projectsState.data }),
  useProject: () => ({ data: null }),
  useProjectStatuses: () => ({ data: null }),
}));

const addToastMock = vi.fn();
vi.mock("../../features/toast/useToast", () => ({
  useToast: () => ({ addToast: addToastMock }),
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
  FlowPill: ({
    phase,
    num,
    den,
    "data-testid": testId,
  }: {
    phase: string;
    num: number;
    den: number;
    "data-testid"?: string;
  }) => (
    <span data-testid={testId ?? "flowpill"} data-phase={phase}>
      {num}/{den}
    </span>
  ),
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

function makeStatus(
  overrides: { key?: string; unblocks_parents?: boolean; unblocks_dependents?: boolean } = {},
) {
  return {
    key: overrides.key ?? "open",
    label: "Open",
    color: "#3498db",
    unblocks_parents: overrides.unblocks_parents ?? false,
    unblocks_dependents: overrides.unblocks_dependents ?? false,
    cascades_to_children: false,
    position: 0,
  };
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
      status: makeStatus(),
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
  treesState.neighborhoodMap = new Map();
  treesState.sessionsByIssue = new Map();
  relationsState.issueIds = null;
  relationsState.isLoading = false;
  projectsState.data = [];
  addToastMock.mockReset();
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

  it("renders FlowPill in 'blocked' phase when any blocker is active", () => {
    const issue = makeIssue("i-blocked", { title: "blocked row" });
    paginatedState.issues = [issue];
    treesState.neighborhoodMap = new Map([
      [
        issue.issue_id,
        {
          blockers: [
            { id: "i-b1", status: makeStatus({ unblocks_dependents: false }) },
            { id: "i-b2", status: makeStatus({ unblocks_dependents: true }) },
          ],
          children: [],
        },
      ],
    ]);

    const { getByTestId } = renderIssuesList("/");

    const pill = getByTestId(`issues-row-flowpill-${issue.issue_id}`);
    expect(pill.getAttribute("data-phase")).toBe("blocked");
    expect(pill.textContent).toBe("1/2");
  });

  it("renders FlowPill in 'progress' phase counting completed children", () => {
    const issue = makeIssue("i-progress", { title: "progress row" });
    paginatedState.issues = [issue];
    treesState.neighborhoodMap = new Map([
      [
        issue.issue_id,
        {
          blockers: [],
          children: [
            { id: "c1", status: makeStatus({ unblocks_parents: true }) },
            { id: "c2", status: makeStatus({ unblocks_parents: false }) },
          ],
        },
      ],
    ]);

    const { getByTestId } = renderIssuesList("/");

    const pill = getByTestId(`issues-row-flowpill-${issue.issue_id}`);
    expect(pill.getAttribute("data-phase")).toBe("progress");
    expect(pill.textContent).toBe("1/2");
  });

  it("renders FlowPill in 'done' phase when every child unblocks the parent", () => {
    const issue = makeIssue("i-done", { title: "done row" });
    paginatedState.issues = [issue];
    treesState.neighborhoodMap = new Map([
      [
        issue.issue_id,
        {
          blockers: [],
          children: [
            { id: "c1", status: makeStatus({ unblocks_parents: true }) },
            { id: "c2", status: makeStatus({ unblocks_parents: true }) },
          ],
        },
      ],
    ]);

    const { getByTestId } = renderIssuesList("/");

    const pill = getByTestId(`issues-row-flowpill-${issue.issue_id}`);
    expect(pill.getAttribute("data-phase")).toBe("done");
    expect(pill.textContent).toBe("2/2");
  });

  it("renders no FlowPill when the issue has no children and no blockers", () => {
    const issue = makeIssue("i-empty", { title: "empty row" });
    paginatedState.issues = [issue];
    treesState.neighborhoodMap = new Map();

    const { queryByTestId } = renderIssuesList("/");

    expect(queryByTestId(`issues-row-flowpill-${issue.issue_id}`)).toBeNull();
  });

  it("prefers blocker phase over child progress when both are present", () => {
    const issue = makeIssue("i-both", { title: "both row" });
    paginatedState.issues = [issue];
    treesState.neighborhoodMap = new Map([
      [
        issue.issue_id,
        {
          blockers: [
            { id: "i-b1", status: makeStatus({ unblocks_dependents: false }) },
          ],
          children: [
            { id: "c1", status: makeStatus({ unblocks_parents: true }) },
          ],
        },
      ],
    ]);

    const { getByTestId } = renderIssuesList("/");

    const pill = getByTestId(`issues-row-flowpill-${issue.issue_id}`);
    expect(pill.getAttribute("data-phase")).toBe("blocked");
    expect(pill.textContent).toBe("1/1");
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

describe("IssuesListPage project URL params", () => {
  // The Issues-list URL splits the project-selection job across two URL
  // params with disjoint value spaces:
  //   - `?project=<j-id>` is the canonical id form.
  //   - `?project_key=<slug>` is the human-friendly slug form, resolved to
  //     a `j-`-prefixed id and rewritten to `?project=` on the next render.
  // This split avoids the prior single-`?project=` parameter, where slug vs.
  // id was disambiguated by string-prefix and could collide silently — see
  // `docs/architecture/api-wire-contract.md` ("Parameter forms must be
  // mutually exclusive by construction").

  it("resolves ?project_key=<slug> and rewrites the URL to canonical ?project=j-<id>", () => {
    projectsState.data = [
      {
        project_id: "j-hidryk",
        project: { key: "engineering-v2", name: "Engineering v2" },
      },
    ];

    const { getByTestId } = renderIssuesList("/?project_key=engineering-v2");

    // URL canonicalized to the id form (server-applicable, copy-paste stable).
    expect(getByTestId("location").textContent).toBe("/?project=j-hidryk");
    // Server fetch sees the resolved id, not the raw slug.
    expect(paginatedState.paginatedFilters).toEqual({
      project_id: "j-hidryk",
    });
  });

  it("drops the project filter and surfaces a toast for an unknown ?project_key=", () => {
    projectsState.data = [
      {
        project_id: "j-hidryk",
        project: { key: "engineering-v2", name: "Engineering v2" },
      },
    ];

    const { getByTestId } = renderIssuesList("/?project_key=does-not-exist");

    // Project filter stripped from the URL — no unresolved value lingers.
    expect(getByTestId("location").textContent).toBe("/");
    // Server never sees the unresolved slug (would 400 the backend).
    expect(paginatedState.paginatedFilters).toEqual({});
    expect(addToastMock).toHaveBeenCalledWith(
      "Unknown project key: does-not-exist",
      "error",
    );
  });

  it("drops the project filter and toasts when ?project= is set to a non-`j-` value", () => {
    // Pasted legacy-style URL: `?project=engineering-v2`. After the split,
    // `?project=` accepts only `j-`-prefixed ids; the slug form belongs in
    // `?project_key=`. The bad URL token is dropped + the user is told.
    projectsState.data = [
      {
        project_id: "j-hidryk",
        project: { key: "engineering-v2", name: "Engineering v2" },
      },
    ];

    const { getByTestId } = renderIssuesList("/?project=engineering-v2");

    expect(getByTestId("location").textContent).toBe("/");
    expect(paginatedState.paginatedFilters).toEqual({});
    expect(addToastMock).toHaveBeenCalledWith(
      "Invalid project URL parameter: engineering-v2",
      "error",
    );
  });

  it("drops + toasts when ?project_key= itself is `j-`-prefixed (wrong value space)", () => {
    projectsState.data = [
      {
        project_id: "j-hidryk",
        project: { key: "engineering-v2", name: "Engineering v2" },
      },
    ];

    const { getByTestId } = renderIssuesList("/?project_key=j-hidryk");

    // `?project_key=` is the slug parameter — a `j-`-prefixed value here is
    // a value-space violation, not silently re-routed to `?project=`. That's
    // the whole point of the split: ambiguity surfaces as an error.
    expect(getByTestId("location").textContent).toBe("/");
    expect(paginatedState.paginatedFilters).toEqual({});
    expect(addToastMock).toHaveBeenCalledWith(
      "Invalid project URL parameter: j-hidryk",
      "error",
    );
  });

  it("passes a `j-`-prefixed ?project= token through unchanged", () => {
    projectsState.data = [
      {
        project_id: "j-hidryk",
        project: { key: "engineering-v2", name: "Engineering v2" },
      },
    ];

    const { getByTestId } = renderIssuesList("/?project=j-hidryk");

    // No URL rewrite when the value already matches the canonical form.
    expect(getByTestId("location").textContent).toBe("/?project=j-hidryk");
    expect(paginatedState.paginatedFilters).toEqual({
      project_id: "j-hidryk",
    });
    expect(addToastMock).not.toHaveBeenCalled();
  });

  it("resolves ?project_key= before sending project+status to the server", () => {
    projectsState.data = [
      {
        project_id: "j-hidryk",
        project: { key: "engineering-v2", name: "Engineering v2" },
      },
    ];

    const { getByTestId } = renderIssuesList(
      "/?project_key=engineering-v2&status=inbox",
    );

    // URL: project canonicalized into the `?project=` slot; status passed
    // through. `?project_key=` is gone — the rewrite consumed it.
    expect(getByTestId("location").textContent).toBe(
      "/?status=inbox&project=j-hidryk",
    );
    // Server fetch carries the resolved id alongside the per-project status key.
    expect(paginatedState.paginatedFilters).toEqual({
      project_id: "j-hidryk",
      status: "inbox",
    });
  });

  it("holds off the server query while ?project_key= is still resolving", () => {
    // Simulate the in-flight projects query: `data` is `undefined`. Without
    // the gate, the first render would fire `listIssues` with no project
    // filter and show every project's issues before the resolver settled.
    projectsState.data = undefined;

    const { getByTestId } = renderIssuesList("/?project_key=engineering-v2");

    // `filtersToIssuesQuery` never sees the slug — it lives on URL only,
    // not in filter state. The server filters are an empty record (the
    // `enabled` gate, asserted via the URL still showing `?project_key=`,
    // is what suppresses the no-op query from actually firing).
    expect(paginatedState.paginatedFilters).toEqual({});
    // URL left as the user pasted it, awaiting the projects list to resolve.
    expect(getByTestId("location").textContent).toBe(
      "/?project_key=engineering-v2",
    );
  });
});
