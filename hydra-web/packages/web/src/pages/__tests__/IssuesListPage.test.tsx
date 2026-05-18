// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, cleanup } from "@testing-library/react";
import React from "react";
import { MemoryRouter, Routes, Route, useLocation } from "react-router-dom";
import type { ChildStatus } from "../../features/dashboard/computeIssueProgress";
import type { IssueSummaryRecord, SessionSummaryRecord } from "@hydra/api";

// --- Mocks ---

vi.mock("../../features/dashboard/HeterogeneousItemList", () => ({
  HeterogeneousItemList: () => <div data-testid="heterogeneous-item-list" />,
}));

vi.mock("../../features/dashboard/FilterBar", () => ({
  FilterBar: () => <div data-testid="filter-bar" />,
}));

interface PaginatedState {
  issues: IssueSummaryRecord[];
}
const paginatedState: PaginatedState = { issues: [] };

vi.mock("../../features/issues/usePaginatedIssues", () => ({
  usePaginatedIssues: () => ({
    data: { pages: [{ issues: paginatedState.issues }] },
    isLoading: false,
    fetchNextPage: vi.fn(),
    hasNextPage: false,
    isFetchingNextPage: false,
  }),
}));

vi.mock("../../features/dashboard/usePaginatedPatches", () => ({
  usePaginatedPatches: () => ({
    data: { pages: [] },
    isLoading: false,
    fetchNextPage: vi.fn(),
    hasNextPage: false,
    isFetchingNextPage: false,
  }),
}));

vi.mock("../../features/dashboard/usePaginatedDocuments", () => ({
  usePaginatedDocuments: () => ({
    data: { pages: [] },
    isLoading: false,
    fetchNextPage: vi.fn(),
    hasNextPage: false,
    isFetchingNextPage: false,
  }),
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
}));

vi.mock("../../features/labels/useLabels", () => ({
  useInboxLabel: () => ({ data: undefined }),
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

vi.mock("../../utils/statusMapping", () => ({
  TERMINAL_STATUSES: new Set(["closed", "failed"]),
  normalizeIssueStatus: (s: string) => s,
}));

vi.mock("../../features/dashboard/filterStorage", () => ({
  readFilterState: () => null,
  writeFilterState: vi.fn(),
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
  Avatar: ({ name }: { name: string }) => <span data-testid="avatar">{name}</span>,
  Badge: ({ status }: { status: string }) => <span data-testid="badge">{status}</span>,
  TypeChip: ({ type }: { type: string }) => <span data-testid="type-chip">{type}</span>,
  Kbd: ({ children }: { children: React.ReactNode }) => <kbd>{children}</kbd>,
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
  treesState.childStatusMap = new Map();
  treesState.sessionsByIssue = new Map();
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
    expect(screen.queryByTestId("issue-create-modal")).toBeNull();
  });
});

describe("IssuesListPage breadcrumb label", () => {
  it("publishes Workspace / Issues breadcrumb on the default view", () => {
    renderIssuesList("/");
    expect(useBreadcrumbsMock).toHaveBeenCalledWith([{ label: "Workspace", to: "/" }], "Issues");
  });

  it("publishes Workspace / Assigned to me when ?selected=assigned", () => {
    renderIssuesList("/?selected=assigned");
    expect(useBreadcrumbsMock).toHaveBeenCalledWith(
      [{ label: "Workspace", to: "/" }],
      "Assigned to me",
    );
  });

  it("normalises legacy ?selected=patches back to the default Issues view", () => {
    renderIssuesList("/?selected=patches");
    // patches is no longer a dashboard tab — fall back to Issues
    expect(useBreadcrumbsMock).toHaveBeenCalledWith([{ label: "Workspace", to: "/" }], "Issues");
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

    renderIssuesList("/");

    const cell = screen.getByTestId("runtime-active");
    expect(cell).toBeDefined();
    expect(cell.className).toContain("runtimeActive");
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

    renderIssuesList("/");

    const cell = screen.getByTestId("runtime-idle");
    expect(cell).toBeDefined();
    expect(cell.className).toContain("runtimeIdle");
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
    // Idle row carries the base progressFill but not the active variant —
    // the active variant is what swaps the fill from green to yellow + glow.
    expect(fills[0]!.className).toContain("progressFill");
    expect(fills[0]!.className).not.toContain("progressFillActive");
    // Active row carries both classes so the active variant overrides the
    // base green background with the yellow in-progress color.
    expect(fills[1]!.className).toContain("progressFill");
    expect(fills[1]!.className).toContain("progressFillActive");
  });
});
