// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
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

vi.mock("../../layout/useBreadcrumbs", () => ({
  useBreadcrumbs: vi.fn(),
}));

const openIssueCreateModalMock = vi.fn();
vi.mock("../../features/dashboard/useIssueCreateModal", () => ({
  useIssueCreateModal: () => ({
    isOpen: false,
    open: openIssueCreateModalMock,
    close: vi.fn(),
  }),
}));

vi.mock("../DashboardPage.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

// --- Import after mocks ---
const { DashboardPage } = await import("../DashboardPage");

function LocationDisplay() {
  const location = useLocation();
  return (
    <div data-testid="location">
      {location.pathname}
      {location.search}
    </div>
  );
}

function renderDashboard(initialEntry: string) {
  return render(
    <MemoryRouter initialEntries={[initialEntry]}>
      <Routes>
        <Route path="/" element={<DashboardPage />} />
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

describe("DashboardPage + Create Issue button", () => {
  it("calls the global issue-create-modal context open() when the dashboard + Create Issue button is clicked", () => {
    renderDashboard("/");
    fireEvent.click(screen.getByRole("button", { name: /create issue/i }));
    expect(openIssueCreateModalMock).toHaveBeenCalledTimes(1);
  });

  it("does not mount its own IssueCreateModal", () => {
    renderDashboard("/?create-issue=1");
    expect(screen.queryByTestId("issue-create-modal")).toBeNull();
  });
});
