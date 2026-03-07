import { describe, it, expect } from "vitest";
import type { IssueSummaryRecord, PatchSummaryRecord } from "@metis/api";
import type { WorkItem } from "../dashboard/useTransitiveWorkItems";
import { topologicalSort, topologicalSortWorkItems } from "./topologicalSort";

function makeRecord(
  id: string,
  status: string,
  dependencies: Array<{ type: string; issue_id: string }> = [],
): IssueSummaryRecord {
  return {
    issue_id: id,
    version: BigInt(1),
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    issue: {
      type: "task",
      title: "",
      description: "",
      creator: "test",
      status: status as IssueSummaryRecord["issue"]["status"],
      progress: "",
      dependencies: dependencies as IssueSummaryRecord["issue"]["dependencies"],
      patches: [],
    },
  };
}

function ids(records: IssueSummaryRecord[]): string[] {
  return records.map((r) => r.issue_id);
}

describe("topologicalSort", () => {
  it("returns empty array for empty input", () => {
    expect(topologicalSort([])).toEqual([]);
  });

  it("returns single issue unchanged", () => {
    const a = makeRecord("a", "open");
    expect(ids(topologicalSort([a]))).toEqual(["a"]);
  });

  it("preserves input order when there are no blocked-on edges", () => {
    const a = makeRecord("a", "open");
    const b = makeRecord("b", "open");
    const c = makeRecord("c", "open");
    expect(ids(topologicalSort([a, b, c]))).toEqual(["a", "b", "c"]);
  });

  it("sorts a linear chain: A blocks B blocks C", () => {
    const a = makeRecord("a", "open");
    const b = makeRecord("b", "open", [
      { type: "blocked-on", issue_id: "a" },
    ]);
    const c = makeRecord("c", "open", [
      { type: "blocked-on", issue_id: "b" },
    ]);
    // Even if input order is reversed, output should be A, B, C
    expect(ids(topologicalSort([c, b, a]))).toEqual(["a", "b", "c"]);
  });

  it("sorts diamond dependencies correctly", () => {
    // A blocks B and C; both B and C block D
    const a = makeRecord("a", "open");
    const b = makeRecord("b", "open", [
      { type: "blocked-on", issue_id: "a" },
    ]);
    const c = makeRecord("c", "open", [
      { type: "blocked-on", issue_id: "a" },
    ]);
    const d = makeRecord("d", "open", [
      { type: "blocked-on", issue_id: "b" },
      { type: "blocked-on", issue_id: "c" },
    ]);
    const result = ids(topologicalSort([d, c, b, a]));
    // A must come before B and C; B and C must come before D
    expect(result.indexOf("a")).toBeLessThan(result.indexOf("b"));
    expect(result.indexOf("a")).toBeLessThan(result.indexOf("c"));
    expect(result.indexOf("b")).toBeLessThan(result.indexOf("d"));
    expect(result.indexOf("c")).toBeLessThan(result.indexOf("d"));
  });

  it("ignores blocked-on edges referencing non-sibling issues", () => {
    const a = makeRecord("a", "open", [
      { type: "blocked-on", issue_id: "external" },
    ]);
    const b = makeRecord("b", "open");
    // "external" is not in the sibling set, so the edge is ignored
    expect(ids(topologicalSort([a, b]))).toEqual(["a", "b"]);
  });

  it("ignores child-of dependencies", () => {
    const a = makeRecord("a", "open", [
      { type: "child-of", issue_id: "b" },
    ]);
    const b = makeRecord("b", "open");
    // child-of should not affect ordering
    expect(ids(topologicalSort([a, b]))).toEqual(["a", "b"]);
  });

  it("handles cycles gracefully by appending remaining in input order", () => {
    const a = makeRecord("a", "open", [
      { type: "blocked-on", issue_id: "b" },
    ]);
    const b = makeRecord("b", "open", [
      { type: "blocked-on", issue_id: "a" },
    ]);
    const c = makeRecord("c", "open");
    // C has no deps so it comes first; A and B form a cycle
    const result = ids(topologicalSort([a, b, c]));
    expect(result[0]).toBe("c");
    // A and B should both be present (appended in input order)
    expect(result).toContain("a");
    expect(result).toContain("b");
    expect(result.length).toBe(3);
  });

  it("preserves input order as tiebreaker within the same topological tier", () => {
    // A and B are both unblocked; C is blocked on both
    const a = makeRecord("a", "open");
    const b = makeRecord("b", "open");
    const c = makeRecord("c", "open", [
      { type: "blocked-on", issue_id: "a" },
      { type: "blocked-on", issue_id: "b" },
    ]);
    // Input order: B, A, C — within tier 0, B should come before A
    expect(ids(topologicalSort([b, a, c]))).toEqual(["b", "a", "c"]);
  });

  it("handles multiple independent chains", () => {
    // Chain 1: X -> Y   Chain 2: P -> Q
    const x = makeRecord("x", "open");
    const y = makeRecord("y", "open", [
      { type: "blocked-on", issue_id: "x" },
    ]);
    const p = makeRecord("p", "open");
    const q = makeRecord("q", "open", [
      { type: "blocked-on", issue_id: "p" },
    ]);
    // Input: q, y, p, x — tiebreaker within tiers preserves input order
    const result = ids(topologicalSort([q, y, p, x]));
    expect(result.indexOf("x")).toBeLessThan(result.indexOf("y"));
    expect(result.indexOf("p")).toBeLessThan(result.indexOf("q"));
  });
});

// ---------------------------------------------------------------------------
// topologicalSortWorkItems tests
// ---------------------------------------------------------------------------

function makeWorkItem(
  id: string,
  dependencies: Array<{ type: string; issue_id: string }> = [],
  lastUpdated = "2026-01-01T00:00:00Z",
): WorkItem {
  return {
    kind: "issue",
    id,
    data: {
      issue_id: id,
      version: BigInt(1),
      timestamp: lastUpdated,
      creation_time: lastUpdated,
      issue: {
        type: "task",
        title: "",
        description: "",
        creator: "test",
        status: "open" as IssueSummaryRecord["issue"]["status"],
        progress: "",
        dependencies:
          dependencies as IssueSummaryRecord["issue"]["dependencies"],
        patches: [],
      },
    },
    lastUpdated,
    isTerminal: false,
  };
}

function makePatchWorkItem(
  id: string,
  lastUpdated = "2026-01-01T00:00:00Z",
): WorkItem {
  return {
    kind: "patch",
    id,
    data: {
      patch_id: id,
      version: BigInt(1),
      timestamp: lastUpdated,
      creation_time: lastUpdated,
      patch: {
        status: "Open",
        title: "",
        is_automatic_backup: false,
        creator: "test",
        review_summary: { count: 0, approved: false },
        service_repo_name: "test/repo",
      },
    } as PatchSummaryRecord,
    lastUpdated,
    isTerminal: false,
    sourceIssueId: undefined,
  };
}

function workItemIds(items: WorkItem[]): string[] {
  return items.map((i) => i.id);
}

describe("topologicalSortWorkItems", () => {
  it("returns empty array for empty input", () => {
    expect(topologicalSortWorkItems([])).toEqual([]);
  });

  it("returns single item unchanged", () => {
    const a = makeWorkItem("a");
    expect(workItemIds(topologicalSortWorkItems([a]))).toEqual(["a"]);
  });

  it("sorts a linear child-of chain: C child-of B child-of A", () => {
    // C is child of B, B is child of A → C completes first, then B, then A
    const a = makeWorkItem("a");
    const b = makeWorkItem("b", [{ type: "child-of", issue_id: "a" }]);
    const c = makeWorkItem("c", [{ type: "child-of", issue_id: "b" }]);
    // Input order reversed; output should be C, B, A (leaf first)
    const result = workItemIds(topologicalSortWorkItems([a, b, c]));
    expect(result.indexOf("c")).toBeLessThan(result.indexOf("b"));
    expect(result.indexOf("b")).toBeLessThan(result.indexOf("a"));
  });

  it("sorts blocked-on: B blocked-on A means A before B", () => {
    const a = makeWorkItem("a");
    const b = makeWorkItem("b", [{ type: "blocked-on", issue_id: "a" }]);
    const result = workItemIds(topologicalSortWorkItems([b, a]));
    expect(result).toEqual(["a", "b"]);
  });

  it("sorts diamond child-of dependencies: leaf first, root last", () => {
    // A is root; B and C are children of A; D is child of both B and C
    const a = makeWorkItem("a");
    const b = makeWorkItem("b", [{ type: "child-of", issue_id: "a" }]);
    const c = makeWorkItem("c", [{ type: "child-of", issue_id: "a" }]);
    const d = makeWorkItem("d", [
      { type: "child-of", issue_id: "b" },
      { type: "child-of", issue_id: "c" },
    ]);
    const result = workItemIds(topologicalSortWorkItems([a, b, c, d]));
    // D must come before B and C; B and C must come before A
    expect(result.indexOf("d")).toBeLessThan(result.indexOf("b"));
    expect(result.indexOf("d")).toBeLessThan(result.indexOf("c"));
    expect(result.indexOf("b")).toBeLessThan(result.indexOf("a"));
    expect(result.indexOf("c")).toBeLessThan(result.indexOf("a"));
  });

  it("handles cycles gracefully without crashing", () => {
    const a = makeWorkItem("a", [{ type: "blocked-on", issue_id: "b" }]);
    const b = makeWorkItem("b", [{ type: "blocked-on", issue_id: "a" }]);
    const c = makeWorkItem("c");
    const result = workItemIds(topologicalSortWorkItems([a, b, c]));
    // C has no deps so it comes first; A and B in cycle are appended
    expect(result[0]).toBe("c");
    expect(result).toContain("a");
    expect(result).toContain("b");
    expect(result.length).toBe(3);
  });

  it("handles mixed child-of and blocked-on dependencies", () => {
    // A is root parent, B is child of A, C is child of A and blocked on B
    const a = makeWorkItem("a");
    const b = makeWorkItem("b", [{ type: "child-of", issue_id: "a" }]);
    const c = makeWorkItem("c", [
      { type: "child-of", issue_id: "a" },
      { type: "blocked-on", issue_id: "b" },
    ]);
    const result = workItemIds(topologicalSortWorkItems([a, c, b]));
    // B completes first (leaf, no blockers), then C (blocked on B), then A (root)
    expect(result.indexOf("b")).toBeLessThan(result.indexOf("c"));
    expect(result.indexOf("c")).toBeLessThan(result.indexOf("a"));
  });

  it("ignores edges to issues not in the active set", () => {
    const a = makeWorkItem("a", [
      { type: "child-of", issue_id: "external" },
      { type: "blocked-on", issue_id: "also-external" },
    ]);
    const b = makeWorkItem("b");
    const result = workItemIds(topologicalSortWorkItems([a, b]));
    // No valid edges, so sorted by lastUpdated (same timestamp = stable)
    expect(result).toEqual(["a", "b"]);
  });

  it("sorts by lastUpdated within the same tier", () => {
    // B and C are both children of A; no deps between B and C
    const a = makeWorkItem("a", [], "2026-01-01T00:00:00Z");
    const b = makeWorkItem(
      "b",
      [{ type: "child-of", issue_id: "a" }],
      "2026-01-01T01:00:00Z",
    );
    const c = makeWorkItem(
      "c",
      [{ type: "child-of", issue_id: "a" }],
      "2026-01-01T02:00:00Z",
    );
    const result = workItemIds(topologicalSortWorkItems([a, b, c]));
    // C has later lastUpdated, so it comes first within the same tier
    expect(result).toEqual(["c", "b", "a"]);
  });

  it("places non-issue items after issue items", () => {
    const issue = makeWorkItem("i1");
    const patch = makePatchWorkItem("p1");
    const result = workItemIds(topologicalSortWorkItems([patch, issue]));
    expect(result).toEqual(["i1", "p1"]);
  });

  it("handles only non-issue items", () => {
    const p1 = makePatchWorkItem("p1", "2026-01-01T01:00:00Z");
    const p2 = makePatchWorkItem("p2", "2026-01-01T02:00:00Z");
    const result = workItemIds(topologicalSortWorkItems([p1, p2]));
    // Sorted by lastUpdated descending
    expect(result).toEqual(["p2", "p1"]);
  });
});
