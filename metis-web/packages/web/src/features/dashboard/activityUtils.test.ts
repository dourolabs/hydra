import { describe, it, expect } from "vitest";
import type { IssueSummaryRecord } from "@metis/api";
import type { IssueTreeNode } from "../issues/useIssues";
import { computeIssueProgress } from "./activityUtils";

function makeRecord(
  id: string,
  status: string,
): IssueSummaryRecord {
  return {
    issue_id: id,
    version: BigInt(1),
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    issue: {
      type: "task",
      description: `Issue ${id}`,
      creator: "test",
      status: status as IssueSummaryRecord["issue"]["status"],
      dependencies: [],
      patches: [],
    },
  };
}

function makeNode(
  id: string,
  status: string,
  children: IssueTreeNode[] = [],
  overrides: Partial<IssueTreeNode> = {},
): IssueTreeNode {
  return {
    id,
    issue: makeRecord(id, status),
    children,
    defaultExpanded: false,
    blocked: false,
    blockedBy: [],
    hardBlocked: false,
    hardBlockedBy: [],
    ...overrides,
  };
}

describe("computeIssueProgress", () => {
  it("returns an empty array for empty input", () => {
    expect(computeIssueProgress([])).toEqual([]);
  });

  it("returns zero counts for a root with no children", () => {
    const root = makeNode("root", "open");
    const result = computeIssueProgress([root]);

    expect(result).toHaveLength(1);
    expect(result[0].open).toBe(0);
    expect(result[0].inProgress).toBe(0);
    expect(result[0].closed).toBe(0);
    expect(result[0].total).toBe(0);
  });

  it("counts children with mixed statuses correctly", () => {
    const root = makeNode("root", "open", [
      makeNode("c1", "open"),
      makeNode("c2", "in-progress"),
      makeNode("c3", "closed"),
      makeNode("c4", "open"),
      makeNode("c5", "closed"),
    ]);
    const result = computeIssueProgress([root]);

    expect(result).toHaveLength(1);
    expect(result[0].open).toBe(2);
    expect(result[0].inProgress).toBe(1);
    expect(result[0].closed).toBe(2);
  });

  it("skips hard-blocked children", () => {
    const root = makeNode("root", "open", [
      makeNode("c1", "open"),
      makeNode("c2", "open", [], { hardBlocked: true }),
      makeNode("c3", "closed"),
    ]);
    const result = computeIssueProgress([root]);

    expect(result).toHaveLength(1);
    expect(result[0].open).toBe(1);
    expect(result[0].closed).toBe(1);
    expect(result[0].total).toBe(2);
  });

  it("does not count failed, dropped, or rejected statuses", () => {
    const root = makeNode("root", "open", [
      makeNode("c1", "open"),
      makeNode("c2", "failed"),
      makeNode("c3", "dropped"),
      makeNode("c4", "rejected"),
      makeNode("c5", "closed"),
    ]);
    const result = computeIssueProgress([root]);

    expect(result[0].open).toBe(1);
    expect(result[0].inProgress).toBe(0);
    expect(result[0].closed).toBe(1);
    expect(result[0].total).toBe(2);
  });

  it("produces independent entries for multiple roots", () => {
    const root1 = makeNode("r1", "open", [
      makeNode("c1", "open"),
      makeNode("c2", "closed"),
    ]);
    const root2 = makeNode("r2", "open", [
      makeNode("c3", "in-progress"),
    ]);
    const result = computeIssueProgress([root1, root2]);

    expect(result).toHaveLength(2);
    expect(result[0].rootId).toBe("r1");
    expect(result[0].open).toBe(1);
    expect(result[0].closed).toBe(1);
    expect(result[1].rootId).toBe("r2");
    expect(result[1].inProgress).toBe(1);
  });

  it("computes total as open + inProgress + closed", () => {
    const root = makeNode("root", "open", [
      makeNode("c1", "open"),
      makeNode("c2", "in-progress"),
      makeNode("c3", "closed"),
      makeNode("c4", "failed"),
      makeNode("c5", "open", [], { hardBlocked: true }),
    ]);
    const result = computeIssueProgress([root]);

    expect(result[0].total).toBe(
      result[0].open + result[0].inProgress + result[0].closed,
    );
    expect(result[0].total).toBe(3);
  });

  it("preserves root metadata in the output", () => {
    const root = makeNode("my-root", "in-progress", [
      makeNode("c1", "open"),
    ]);
    const result = computeIssueProgress([root]);

    expect(result[0].rootId).toBe("my-root");
    expect(result[0].rootIssue).toBe(root.issue);
  });
});
