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

interface HookState {
  data: SessionSummaryRecord[] | undefined;
  isLoading: boolean;
  error: Error | null;
}

const hookState: HookState = {
  data: undefined,
  isLoading: false,
  error: null,
};
const mockRefetch = vi.fn();

vi.mock("../../features/sessions/useAllSessions", () => ({
  useAllSessions: () => ({
    data: hookState.data,
    isLoading: hookState.isLoading,
    error: hookState.error,
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
  formatTimestamp: (ts: string) => `formatted(${ts})`,
}));

vi.mock("../SessionsListPage.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

// --- Import after mocks ---
const { SessionsListPage } = await import("../SessionsListPage");

// --- Helpers ---

function makeRecord(
  id: string,
  partial: Partial<SessionSummaryRecord["session"]> = {},
  spawnedFrom?: string,
): SessionSummaryRecord {
  return {
    session_id: id,
    version: 1n,
    timestamp: "2026-01-01T00:00:00.000Z",
    session: {
      prompt: "Do the thing",
      creator: "swe",
      status: "running",
      spawned_from: spawnedFrom,
      ...partial,
    },
  };
}

function resetState() {
  hookState.data = undefined;
  hookState.isLoading = false;
  hookState.error = null;
  mockRefetch.mockReset();
}

// --- Tests ---

describe("SessionsListPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetState();
  });

  it("renders the loading state while the query is loading", () => {
    hookState.isLoading = true;
    render(<SessionsListPage />);

    expect(screen.getByTestId("loading")).toBeDefined();
    cleanup();
  });

  it("renders the empty state when there are no sessions", () => {
    hookState.data = [];
    render(<SessionsListPage />);

    expect(screen.getByTestId("empty-state").textContent).toContain(
      "No sessions yet.",
    );
    cleanup();
  });

  it("renders an error state when the query fails", () => {
    hookState.error = new Error("boom");
    render(<SessionsListPage />);

    expect(screen.getByTestId("error-state").textContent).toContain("boom");
    cleanup();
  });

  it("renders a row per session and links rows with a spawned_from issue", () => {
    hookState.data = [
      makeRecord(
        "s-1",
        {
          status: "running",
          start_time: "2026-03-15T10:00:00.000Z",
          creation_time: "2026-03-15T09:59:00.000Z",
        },
        "i-1",
      ),
      makeRecord(
        "s-2",
        {
          status: "complete",
          end_time: "2026-03-14T12:00:00.000Z",
          creation_time: "2026-03-14T10:00:00.000Z",
        },
        undefined,
      ),
    ];

    render(<SessionsListPage />);

    expect(screen.getByTestId("session-row-s-1")).toBeDefined();
    expect(screen.getByTestId("session-row-s-2")).toBeDefined();

    // Active session (s-1) renders before the terminal one (s-2)
    const list = screen.getByTestId("sessions-list");
    const orderedIds = Array.from(list.querySelectorAll("li")).map((li) =>
      li.getAttribute("data-testid"),
    );
    expect(orderedIds).toEqual(["session-row-s-1", "session-row-s-2"]);

    // s-1 has spawned_from → session id is a link to the logs route
    const s1Link = screen
      .getByTestId("session-row-s-1")
      .querySelector("a[href]");
    expect(s1Link?.getAttribute("href")).toBe(
      "/issues/i-1/sessions/s-1/logs",
    );

    // s-2 has no spawned_from → session id rendered as plain text (no link)
    const s2Row = screen.getByTestId("session-row-s-2");
    expect(s2Row.textContent).toContain("s-2");
    // No anchor inside s-2 (no issue link, no session id link)
    expect(s2Row.querySelectorAll("a").length).toBe(0);

    cleanup();
  });
});
