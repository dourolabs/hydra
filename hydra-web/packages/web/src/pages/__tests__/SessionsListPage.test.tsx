import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, cleanup } from "@testing-library/react";
import React from "react";
import type { SessionSummaryRecord } from "@hydra/api";

// --- Mocks ---

vi.mock("react-router-dom", () => ({
  Link: ({
    to,
    children,
    className,
  }: {
    to: string;
    children: React.ReactNode;
    className?: string;
  }) => (
    <a href={to} className={className}>
      {children}
    </a>
  ),
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
const mockRefetch = vi.fn();

vi.mock("../../features/sessions/useAllSessions", () => ({
  useAllSessions: () => ({
    data: allSessionsState.data,
    isLoading: allSessionsState.isLoading,
    error: allSessionsState.error,
    refetch: mockRefetch,
  }),
}));

vi.mock("@hydra/ui", () => ({
  Badge: ({ status }: { status: string }) => (
    <span data-testid="badge">{status}</span>
  ),
}));

vi.mock("../../components/LoadingState/LoadingState", () => ({
  LoadingState: () => <div data-testid="loading" />,
}));

vi.mock("../../components/ErrorState/ErrorState", () => ({
  ErrorState: ({ message }: { message: string }) => (
    <div data-testid="error-state">{message}</div>
  ),
}));

vi.mock("../../components/EmptyState/EmptyState", () => ({
  EmptyState: ({ message }: { message: string }) => (
    <div data-testid="empty-state">{message}</div>
  ),
}));

vi.mock("../../utils/statusMapping", () => ({
  normalizeSessionStatus: (s: string) => s,
}));

vi.mock("../../utils/time", () => ({
  formatTimestamp: (s: string) => s,
}));

vi.mock("../../utils/text", () => ({
  descriptionSnippet: (s: string) => s,
}));

vi.mock("../SessionsListPage.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
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
  mockRefetch.mockReset();
}

describe("SessionsListPage", () => {
  beforeEach(() => {
    reset();
    cleanup();
  });

  it("renders the loading state while data is loading", () => {
    allSessionsState.isLoading = true;
    render(<SessionsListPage />);
    expect(screen.getByTestId("loading")).toBeDefined();
  });

  it("renders the empty state when no sessions exist", () => {
    allSessionsState.data = [];
    render(<SessionsListPage />);
    expect(screen.getByTestId("empty-state").textContent).toContain(
      "No sessions yet.",
    );
  });

  it("renders an error state with the message and a retry handler", () => {
    allSessionsState.error = new Error("boom");
    render(<SessionsListPage />);
    expect(screen.getByTestId("error-state").textContent).toContain("boom");
  });

  it("renders one row per session and links sessions with spawned_from", () => {
    allSessionsState.data = [
      rec("t-1", "running", "i-1", "first task"),
      rec("t-2", "complete", undefined, "orphan task"),
    ];

    render(<SessionsListPage />);

    expect(screen.getByTestId("sessions-list-row-t-1")).toBeDefined();
    expect(screen.getByTestId("sessions-list-row-t-2")).toBeDefined();

    // Linked session id for t-1 (spawned_from i-1)
    const linkedSessionId = screen.getByText("t-1");
    expect(linkedSessionId.closest("a")?.getAttribute("href")).toBe(
      "/issues/i-1/sessions/t-1/logs",
    );

    // Unlinked session id for t-2 — rendered as text, not an anchor.
    const orphanSessionId = screen.getByText("t-2");
    expect(orphanSessionId.tagName.toLowerCase()).not.toBe("a");
    expect(orphanSessionId.closest("a")).toBeNull();

    // Issue link is present for spawned-from sessions.
    const issueLink = screen.getByText("i-1");
    expect(issueLink.closest("a")?.getAttribute("href")).toBe("/issues/i-1");
  });

  it("orders active sessions before terminal sessions", () => {
    allSessionsState.data = [
      rec("term", "complete"),
      rec("active", "running"),
    ];

    render(<SessionsListPage />);
    const items = screen.getAllByText(/^(active|term)$/);
    expect(items[0].textContent).toBe("active");
    expect(items[1].textContent).toBe("term");
  });
});
