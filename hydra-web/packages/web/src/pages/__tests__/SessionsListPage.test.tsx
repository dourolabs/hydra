// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, cleanup } from "@testing-library/react";
import React from "react";
import type { SessionSummaryRecord } from "@hydra/api";

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

interface AllSessionsState {
  data: SessionSummaryRecord[] | undefined;
  isLoading: boolean;
  error: Error | null;
}

const allSessionsState: AllSessionsState = {
  data: undefined,
  isLoading: false,
  error: null,
};

vi.mock("../../features/sessions/useAllSessions", () => ({
  useAllSessions: () => ({
    data: allSessionsState.data,
    isLoading: allSessionsState.isLoading,
    error: allSessionsState.error,
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

function reset() {
  allSessionsState.data = undefined;
  allSessionsState.isLoading = false;
  allSessionsState.error = null;
  navigateMock.mockReset();
}

describe("SessionsListPage", () => {
  beforeEach(() => {
    reset();
    useBreadcrumbsMock.mockReset();
    cleanup();
  });

  it("publishes a Workspace / Sessions breadcrumb on mount", () => {
    allSessionsState.data = [];
    render(<SessionsListPage />);
    expect(useBreadcrumbsMock).toHaveBeenCalledWith(
      [{ label: "Workspace", to: "/" }],
      "Sessions",
    );
  });

  it("shows a loading message before data arrives", () => {
    allSessionsState.isLoading = true;
    render(<SessionsListPage />);
    expect(screen.getByText(/loading sessions/i)).toBeDefined();
  });

  it("shows an empty message when no sessions exist", () => {
    allSessionsState.data = [];
    render(<SessionsListPage />);
    expect(screen.getByText(/no sessions match the current filters/i)).toBeDefined();
  });

  it("shows an error message when the query fails", () => {
    allSessionsState.error = new Error("boom");
    render(<SessionsListPage />);
    expect(screen.getByText(/failed to load sessions/i)).toBeDefined();
  });

  it("renders one row per session, links spawned-from issue, no ID column", () => {
    allSessionsState.data = [
      rec("t-1", "running", "i-1", "first task"),
      rec("t-2", "complete", undefined, "orphan task"),
    ];

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
    allSessionsState.data = [
      rec("term", "complete"),
      rec("active", "running"),
    ];

    render(<SessionsListPage />);

    const rows = screen.getAllByTestId(/^sessions-list-row-/);
    // useAllSessions hook + sortSessions util order active before terminal.
    expect(rows[0].getAttribute("data-testid")).toBe("sessions-list-row-active");
    expect(rows[1].getAttribute("data-testid")).toBe("sessions-list-row-term");
  });
});
