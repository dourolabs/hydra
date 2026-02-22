import { describe, it, expect } from "vitest";
import type { IssueSummaryRecord } from "@metis/api";
import { topologicalSort } from "./topologicalSort";

function makeRecord(
  id: string,
  status: string,
  dependencies: Array<{ type: string; issue_id: string }> = [],
): IssueSummaryRecord {
  return {
    issue_id: id,
    version: BigInt(1),
    timestamp: "2026-01-01T00:00:00Z",
    issue: {
      type: "task",
      description: "",
      creator: "test",
      status: status as IssueSummaryRecord["issue"]["status"],
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
