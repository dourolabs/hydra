import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type { IssueSummaryRecord } from "@hydra/api";

const mockListIssues = vi.fn();

vi.mock("../../../api/client", () => ({
  apiClient: {
    listIssues: (...args: unknown[]) => mockListIssues(...args),
  },
}));

const { useChatTopLevelIssues } = await import("../useChatTopLevelIssues");

function makeIssue(
  issueId: string,
  overrides: { dependencies?: IssueSummaryRecord["issue"]["dependencies"]; timestamp?: string } = {},
): IssueSummaryRecord {
  return {
    issue_id: issueId,
    version: 1n,
    timestamp: overrides.timestamp ?? "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    issue: {
      type: "task",
      title: `Issue ${issueId}`,
      description: "desc",
      creator: "alice",
      status: "open",
      progress: "",
      dependencies: overrides.dependencies ?? [],
      patches: [],
      labels: [],
    },
  };
}

function wrapper({ children }: { children: React.ReactNode }) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

describe("useChatTopLevelIssues", () => {
  beforeEach(() => {
    mockListIssues.mockReset();
  });

  it("filters out issues with a child-of dependency", async () => {
    mockListIssues.mockResolvedValue({
      issues: [
        makeIssue("i-root1"),
        makeIssue("i-child", {
          dependencies: [{ type: "child-of", issue_id: "i-root1" }],
        }),
        makeIssue("i-root2"),
      ],
    });

    const { result } = renderHook(() => useChatTopLevelIssues(new Set()), {
      wrapper,
    });

    await waitFor(() => {
      expect(result.current.issues.map((i) => i.issue_id).sort()).toEqual([
        "i-root1",
        "i-root2",
      ]);
    });
  });

  it("excludes ids passed via excludeIds", async () => {
    mockListIssues.mockResolvedValue({
      issues: [makeIssue("i-a"), makeIssue("i-b"), makeIssue("i-c")],
    });

    const { result } = renderHook(
      () => useChatTopLevelIssues(new Set(["i-b"])),
      { wrapper },
    );

    await waitFor(() => {
      expect(result.current.issues.map((i) => i.issue_id)).not.toContain("i-b");
      expect(result.current.issues.map((i) => i.issue_id).sort()).toEqual([
        "i-a",
        "i-c",
      ]);
    });
  });

  it("sorts results by timestamp descending", async () => {
    mockListIssues.mockResolvedValue({
      issues: [
        makeIssue("i-old", { timestamp: "2026-01-01T00:00:00Z" }),
        makeIssue("i-new", { timestamp: "2026-03-01T00:00:00Z" }),
        makeIssue("i-mid", { timestamp: "2026-02-01T00:00:00Z" }),
      ],
    });

    const { result } = renderHook(() => useChatTopLevelIssues(new Set()), {
      wrapper,
    });

    await waitFor(() => {
      expect(result.current.issues.map((i) => i.issue_id)).toEqual([
        "i-new",
        "i-mid",
        "i-old",
      ]);
    });
  });
});
