import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type {
  IssueSummaryRecord,
  SessionSummaryRecord,
} from "@hydra/api";

const mockListSessions = vi.fn();
const mockListIssues = vi.fn();

vi.mock("../../../api/client", () => ({
  apiClient: {
    listSessions: (...args: unknown[]) => mockListSessions(...args),
    listIssues: (...args: unknown[]) => mockListIssues(...args),
  },
}));

const { useChatActiveSessionIssues } = await import(
  "../useChatActiveSessionIssues"
);

function makeIssue(issueId: string): IssueSummaryRecord {
  return {
    issue_id: issueId,
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    issue: {
      type: "task",
      title: `Issue ${issueId}`,
      description: "desc",
      creator: "alice",
      status: "in-progress",
      progress: "",
      dependencies: [],
      patches: [],
      labels: [],
    },
  };
}

function makeSession(
  sessionId: string,
  issueId: string,
  timestamp = "2026-01-01T00:00:00Z",
): SessionSummaryRecord {
  return {
    session_id: sessionId,
    version: 1n,
    timestamp,
    session: {
      prompt: "do work",
      spawned_from: issueId,
      creator: "alice",
      status: "running",
    },
  };
}

function wrapper({ children }: { children: React.ReactNode }) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

describe("useChatActiveSessionIssues", () => {
  beforeEach(() => {
    mockListSessions.mockReset();
    mockListIssues.mockReset();
  });

  it("requests sessions with status=running,pending and limit=100", async () => {
    mockListSessions.mockResolvedValue({ sessions: [] });

    renderHook(() => useChatActiveSessionIssues(), { wrapper });

    await waitFor(() => {
      expect(mockListSessions).toHaveBeenCalledWith({
        status: "running,pending",
        limit: 100,
      });
    });
  });

  it("dedups sessions by spawned_from and orders by most recent timestamp", async () => {
    mockListSessions.mockResolvedValue({
      sessions: [
        makeSession("s-a", "i-1", "2026-01-01T00:00:00Z"),
        makeSession("s-b", "i-1", "2026-02-01T00:00:00Z"),
        makeSession("s-c", "i-2", "2026-03-01T00:00:00Z"),
        makeSession("s-d", "i-3", "2026-01-15T00:00:00Z"),
      ],
    });
    mockListIssues.mockImplementation(({ ids }: { ids: string }) => {
      const idList = ids.split(",");
      return Promise.resolve({
        issues: idList.map(makeIssue),
      });
    });

    const { result } = renderHook(() => useChatActiveSessionIssues(), {
      wrapper,
    });

    await waitFor(() => {
      expect(result.current.issues.map((i) => i.issue_id)).toEqual([
        "i-2",
        "i-1",
        "i-3",
      ]);
    });

    expect(result.current.sessionsByIssue.get("i-1")?.length).toBe(2);
  });

  it("skips sessions with no spawned_from", async () => {
    mockListSessions.mockResolvedValue({
      sessions: [
        {
          session_id: "s-orphan",
          version: 1n,
          timestamp: "2026-01-01T00:00:00Z",
          session: {
            prompt: "p",
            creator: "alice",
            status: "running",
          },
        },
        makeSession("s-a", "i-1"),
      ],
    });
    mockListIssues.mockImplementation(({ ids }: { ids: string }) =>
      Promise.resolve({ issues: ids.split(",").map(makeIssue) }),
    );

    const { result } = renderHook(() => useChatActiveSessionIssues(), {
      wrapper,
    });

    await waitFor(() => {
      expect(result.current.issues.map((i) => i.issue_id)).toEqual(["i-1"]);
    });
  });
});
