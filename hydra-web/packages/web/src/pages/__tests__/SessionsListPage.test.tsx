// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, cleanup } from "@testing-library/react";
import React from "react";
import type { ListSessionsResponse, SessionSummaryRecord } from "@hydra/api";

// --- Mocks ---

const navigateMock = vi.fn();
vi.mock("react-router-dom", () => ({
  Link: ({
    to,
    children,
    className,
    onClick,
  }: {
    to: string;
    children: React.ReactNode;
    className?: string;
    onClick?: (e: React.MouseEvent) => void;
  }) => (
    <a href={to} className={className} onClick={onClick}>
      {children}
    </a>
  ),
  useNavigate: () => navigateMock,
}));

interface PaginatedSessionsState {
  pages: ListSessionsResponse[] | undefined;
  isLoading: boolean;
  error: Error | null;
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
}

const paginatedState: PaginatedSessionsState = {
  pages: undefined,
  isLoading: false,
  error: null,
  hasNextPage: false,
  isFetchingNextPage: false,
};

const fetchNextPageMock = vi.fn();

const sessionCountState: { count: number | undefined } = { count: undefined };

vi.mock("../../features/sessions/usePaginatedSessions", () => ({
  usePaginatedSessions: () => ({
    data: paginatedState.pages ? { pages: paginatedState.pages } : undefined,
    isLoading: paginatedState.isLoading,
    error: paginatedState.error,
    fetchNextPage: fetchNextPageMock,
    hasNextPage: paginatedState.hasNextPage,
    isFetchingNextPage: paginatedState.isFetchingNextPage,
  }),
  useSessionCount: () => ({
    data: sessionCountState.count,
  }),
}));

vi.mock("@hydra/ui", () => ({
  Avatar: ({ name }: { name: string }) => <span data-testid="avatar">{name}</span>,
  Badge: ({ status }: { status: string }) => (
    <span data-testid="badge">{status}</span>
  ),
}));

vi.mock("../../utils/statusMapping", () => ({
  normalizeSessionStatus: (s: string) => s,
}));

vi.mock("../../utils/time", () => ({
  getRuntime: () => "—",
}));

vi.mock("../../utils/text", () => ({
  descriptionSnippet: (s: string) => s,
}));

vi.mock("../../features/sessions/view/SessionsView.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const useBreadcrumbsMock = vi.fn();
vi.mock("../../layout/useBreadcrumbs", () => ({
  useBreadcrumbs: (...args: unknown[]) => useBreadcrumbsMock(...args),
}));

// --- Import after mocks ---
const { SessionsListPage } = await import("../SessionsListPage");

// --- Helpers ---

function rec(
  id: string,
  status: SessionSummaryRecord["session"]["status"],
  spawnedFrom?: string,
  prompt = "do the thing",
): SessionSummaryRecord {
  return {
    session_id: id,
    version: 1n,
    timestamp: "2026-03-15T10:00:00.000Z",
    session: {
      prompt,
      creator: "swe",
      status,
      spawned_from: spawnedFrom,
      start_time: "2026-03-15T10:00:00.000Z",
      end_time: status === "complete" ? "2026-03-15T11:00:00.000Z" : null,
    },
  };
}

function setSessions(sessions: SessionSummaryRecord[]) {
  paginatedState.pages = [{ sessions }];
}

function reset() {
  paginatedState.pages = undefined;
  paginatedState.isLoading = false;
  paginatedState.error = null;
  paginatedState.hasNextPage = false;
  paginatedState.isFetchingNextPage = false;
  sessionCountState.count = undefined;
  navigateMock.mockReset();
  fetchNextPageMock.mockReset();
}

describe("SessionsListPage", () => {
  beforeEach(() => {
    reset();
    useBreadcrumbsMock.mockReset();
    cleanup();
  });

  it("publishes a Workspace / Sessions breadcrumb on mount", () => {
    setSessions([]);
    render(<SessionsListPage />);
    expect(useBreadcrumbsMock).toHaveBeenCalledWith(
      [{ label: "Workspace", to: "/" }],
      "Sessions",
    );
  });

  it("shows a loading message before data arrives", () => {
    paginatedState.isLoading = true;
    render(<SessionsListPage />);
    expect(screen.getByText(/loading sessions/i)).toBeDefined();
  });

  it("shows an empty message when no sessions exist", () => {
    setSessions([]);
    render(<SessionsListPage />);
    expect(screen.getByText(/no sessions match the current filters/i)).toBeDefined();
  });

  it("shows an error message when the query fails", () => {
    paginatedState.error = new Error("boom");
    render(<SessionsListPage />);
    expect(screen.getByText(/failed to load sessions/i)).toBeDefined();
  });

  it("renders one row per session, links spawned-from issue, no ID column", () => {
    setSessions([
      rec("t-1", "running", "i-1", "first task"),
      rec("t-2", "complete", undefined, "orphan task"),
    ]);

    render(<SessionsListPage />);

    expect(screen.getByTestId("sessions-list-row-t-1")).toBeDefined();
    expect(screen.getByTestId("sessions-list-row-t-2")).toBeDefined();

    // Per "no IDs on list views" rule: session_id text is NOT shown directly.
    expect(screen.queryByText("t-1")).toBeNull();
    expect(screen.queryByText("t-2")).toBeNull();

    // The spawned_from issue ID is still rendered as a link.
    const issueLink = screen.getByText("i-1");
    expect(issueLink.closest("a")?.getAttribute("href")).toBe("/issues/i-1");
  });

  it("orders active sessions before terminal sessions", () => {
    setSessions([
      rec("term", "complete"),
      rec("active", "running"),
    ]);

    render(<SessionsListPage />);

    const rows = screen.getAllByTestId(/^sessions-list-row-/);
    // sortSessions util orders active before terminal.
    expect(rows[0].getAttribute("data-testid")).toBe("sessions-list-row-active");
    expect(rows[1].getAttribute("data-testid")).toBe("sessions-list-row-term");
  });

  it("deduplicates sessions appearing in multiple pages", () => {
    paginatedState.pages = [
      { sessions: [rec("t-1", "running")] },
      { sessions: [rec("t-1", "running"), rec("t-2", "running")] },
    ];

    render(<SessionsListPage />);

    expect(screen.getAllByTestId(/^sessions-list-row-/).length).toBe(2);
  });

  it("renders Load more when hasNextPage is true and invokes fetchNextPage on click", () => {
    setSessions([rec("t-1", "running")]);
    paginatedState.hasNextPage = true;

    render(<SessionsListPage />);

    const btn = screen.getByTestId("sessions-load-more") as HTMLButtonElement;
    expect(btn).toBeDefined();
    expect(btn.disabled).toBe(false);
    btn.click();
    expect(fetchNextPageMock).toHaveBeenCalledTimes(1);
  });

  it("hides Load more when there is no next page", () => {
    setSessions([rec("t-1", "running")]);
    paginatedState.hasNextPage = false;

    render(<SessionsListPage />);

    expect(screen.queryByTestId("sessions-load-more")).toBeNull();
  });

  it("uses the total count hook value in the eyebrow when available", () => {
    setSessions([rec("t-1", "running")]);
    sessionCountState.count = 1234;

    render(<SessionsListPage />);

    expect(screen.getByText(/1234 SESSIONS/)).toBeDefined();
  });

  it("falls back to row count in the eyebrow when total count is not available", () => {
    setSessions([rec("t-1", "running"), rec("t-2", "complete")]);
    sessionCountState.count = undefined;

    render(<SessionsListPage />);

    expect(screen.getByText(/2 SESSIONS/)).toBeDefined();
  });
});
