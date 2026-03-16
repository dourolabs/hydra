import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type { IssueSummaryRecord } from "@metis/api";

// --- Mocks ---

const mockListRelations = vi.fn();
const mockListIssues = vi.fn();

vi.mock("../../api/client", () => ({
  apiClient: {
    listRelations: (...args: unknown[]) => mockListRelations(...args),
    listIssues: (...args: unknown[]) => mockListIssues(...args),
  },
}));

// --- Helpers ---

function makeWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return ({ children }: { children: React.ReactNode }) =>
    React.createElement(QueryClientProvider, { client: queryClient }, children);
}

function makeIssue(id: string): IssueSummaryRecord {
  return {
    issue_id: id,
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    issue: {
      type: "task",
      title: `Issue ${id}`,
      description: "",
      creator: "alice",
      status: "open",
      progress: "",
      dependencies: [],
      patches: [],
      labels: [],
    },
  } as IssueSummaryRecord;
}

// --- Import after mocks ---
const { useSubtreeIssues } = await import("./useSubtreeIssues");

// --- Tests ---

describe("useSubtreeIssues", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("returns undefined and does not fetch when rootIssueId is null", async () => {
    const { result } = renderHook(() => useSubtreeIssues(null), {
      wrapper: makeWrapper(),
    });

    // Wait a tick to ensure no queries fire
    await new Promise((r) => setTimeout(r, 50));

    expect(result.current.data).toBeUndefined();
    expect(mockListRelations).not.toHaveBeenCalled();
    expect(mockListIssues).not.toHaveBeenCalled();
  });

  it("fetches relations then batch-fetches issues", async () => {
    const rootId = "i-root";
    const childId = "i-child1";
    const grandchildId = "i-child2";

    mockListRelations.mockResolvedValue({
      relations: [
        { source_id: childId, target_id: rootId, rel_type: "child-of" },
        { source_id: grandchildId, target_id: childId, rel_type: "child-of" },
      ],
    });

    const allIssues = [makeIssue(rootId), makeIssue(childId), makeIssue(grandchildId)];
    mockListIssues.mockResolvedValue({ issues: allIssues });

    const { result } = renderHook(() => useSubtreeIssues(rootId), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.data).toHaveLength(3);
    });

    expect(mockListRelations).toHaveBeenCalledWith({
      target_ids: rootId,
      rel_type: "child-of",
      transitive: true,
    });

    // listIssues is called initially with just root, then again with all IDs after relations load.
    // Find the call that includes descendants.
    const allCalls = mockListIssues.mock.calls.map((c) => c[0] as unknown as { ids: string });
    const fullCall = allCalls.find((a) => a.ids.includes(childId));
    expect(fullCall).toBeDefined();
    const calledIds = fullCall!.ids.split(",");
    expect(calledIds).toContain(rootId);
    expect(calledIds).toContain(childId);
    expect(calledIds).toContain(grandchildId);
  });

  it("deduplicates descendant IDs", async () => {
    const rootId = "i-root";
    const childId = "i-dup";

    // Return duplicate source_ids
    mockListRelations.mockResolvedValue({
      relations: [
        { source_id: childId, target_id: rootId, rel_type: "child-of" },
        { source_id: childId, target_id: rootId, rel_type: "child-of" },
      ],
    });

    mockListIssues.mockResolvedValue({
      issues: [makeIssue(rootId), makeIssue(childId)],
    });

    const { result } = renderHook(() => useSubtreeIssues(rootId), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.data).toBeDefined();
    });

    // Find the call that includes the child (after relations loaded)
    const allCalls = mockListIssues.mock.calls.map((c) => c[0] as unknown as { ids: string });
    const fullCall = allCalls.find((a) => a.ids.includes(childId));
    expect(fullCall).toBeDefined();
    const calledIds = fullCall!.ids.split(",");
    // rootId + one deduplicated childId = 2
    expect(calledIds).toHaveLength(2);
    expect(calledIds).toContain(rootId);
    expect(calledIds).toContain(childId);
  });

  it("handles empty relations (leaf node)", async () => {
    const rootId = "i-leaf";

    mockListRelations.mockResolvedValue({ relations: [] });
    mockListIssues.mockResolvedValue({ issues: [makeIssue(rootId)] });

    const { result } = renderHook(() => useSubtreeIssues(rootId), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.data).toBeDefined();
    });

    // Only the root issue should be fetched
    const callArgs = mockListIssues.mock.calls[0][0];
    const calledIds = callArgs.ids.split(",");
    expect(calledIds).toEqual([rootId]);

    expect(result.current.data).toHaveLength(1);
    expect(result.current.data![0].issue_id).toBe(rootId);
  });
});
