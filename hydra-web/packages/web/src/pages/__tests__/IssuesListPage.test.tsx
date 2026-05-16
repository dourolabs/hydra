// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, cleanup } from "@testing-library/react";
import { MemoryRouter, Routes, Route, useLocation } from "react-router-dom";

// --- Mocks ---

vi.mock("../../features/dashboard/HeterogeneousItemList", () => ({
  HeterogeneousItemList: () => <div data-testid="heterogeneous-item-list" />,
}));

vi.mock("../../features/dashboard/FilterBar", () => ({
  FilterBar: () => <div data-testid="filter-bar" />,
}));

vi.mock("../../features/issues/usePaginatedIssues", () => ({
  usePaginatedIssues: () => ({
    data: { pages: [] },
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

vi.mock("../../features/dashboard/usePageIssueTrees", () => ({
  usePageIssueTrees: () => ({
    isActiveMap: new Map(),
    childStatusMap: new Map(),
    sessionsByIssue: new Map(),
    isLoading: false,
  }),
}));

vi.mock("../../utils/statusMapping", () => ({
  TERMINAL_STATUSES: new Set(["closed", "failed"]),
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

beforeEach(() => {
  vi.clearAllMocks();
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
    expect(useBreadcrumbsMock).toHaveBeenCalledWith(
      [{ label: "Workspace", to: "/" }],
      "Issues",
    );
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
    expect(useBreadcrumbsMock).toHaveBeenCalledWith(
      [{ label: "Workspace", to: "/" }],
      "Issues",
    );
  });
});
