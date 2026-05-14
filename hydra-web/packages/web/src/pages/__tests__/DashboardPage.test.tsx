// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import React from "react";
import { MemoryRouter, Routes, Route, useLocation } from "react-router-dom";

// --- Mocks ---

vi.mock("@hydra/ui", () => ({
  Modal: ({
    open,
    title,
    onClose,
    children,
  }: {
    open: boolean;
    title?: string;
    onClose: () => void;
    children: React.ReactNode;
  }) =>
    open ? (
      <div role="dialog" aria-label={title} data-testid="issue-create-modal">
        <button aria-label="Close" onClick={onClose}>
          Close
        </button>
        {children}
      </div>
    ) : null,
  Button: ({
    children,
    onClick,
  }: {
    children: React.ReactNode;
    onClick?: () => void;
  }) => <button onClick={onClick}>{children}</button>,
  Input: () => <input />,
  Textarea: () => <textarea />,
  Select: () => <select />,
  Spinner: () => <div data-testid="spinner" />,
}));

vi.mock("../../features/dashboard/HeterogeneousItemList", () => ({
  HeterogeneousItemList: () => <div data-testid="heterogeneous-item-list" />,
}));

vi.mock("../../features/dashboard/FilterBar", () => ({
  FilterBar: () => <div data-testid="filter-bar" />,
}));

vi.mock("../../features/dashboard/IssueCreateModal", () => ({
  IssueCreateModal: ({
    open,
    onClose,
  }: {
    open: boolean;
    onClose: () => void;
    assignees: string[];
  }) =>
    open ? (
      <div data-testid="issue-create-modal">
        <button data-testid="modal-close" onClick={onClose}>
          Close
        </button>
      </div>
    ) : null,
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

vi.mock("../../hooks/useAgents", () => ({
  useAgents: () => ({ data: [] }),
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
});

describe("DashboardPage create-issue query param", () => {
  it("opens the IssueCreateModal when ?create-issue=1 is present on mount", () => {
    renderDashboard("/?create-issue=1");
    expect(screen.getByTestId("issue-create-modal")).toBeTruthy();
  });

  it("does NOT open the modal when ?create-issue is absent", () => {
    renderDashboard("/");
    expect(screen.queryByTestId("issue-create-modal")).toBeNull();
  });

  it("closes the modal AND removes the create-issue param when the modal is closed", () => {
    renderDashboard("/?create-issue=1");
    expect(screen.getByTestId("issue-create-modal")).toBeTruthy();
    fireEvent.click(screen.getByTestId("modal-close"));
    expect(screen.queryByTestId("issue-create-modal")).toBeNull();
    const location = screen.getByTestId("location").textContent ?? "";
    expect(location.includes("create-issue")).toBe(false);
  });
});

describe("DashboardPage breadcrumb label", () => {
  it("publishes 'Issues' breadcrumb on the default view", () => {
    renderDashboard("/");
    expect(useBreadcrumbsMock).toHaveBeenCalledWith([], "Issues");
    expect(useBreadcrumbsMock).not.toHaveBeenCalledWith([], "Patches");
  });

  it("publishes 'Issues' breadcrumb when selected is a non-patches tab", () => {
    renderDashboard("/?selected=assigned");
    expect(useBreadcrumbsMock).toHaveBeenCalledWith([], "Issues");
    expect(useBreadcrumbsMock).not.toHaveBeenCalledWith([], "Patches");
  });

  it("publishes 'Patches' breadcrumb when ?selected=patches", () => {
    renderDashboard("/?selected=patches");
    expect(useBreadcrumbsMock).toHaveBeenCalledWith([], "Patches");
    expect(useBreadcrumbsMock).not.toHaveBeenCalledWith([], "Issues");
  });
});
