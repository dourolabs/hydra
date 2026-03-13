import { describe, expect, it } from "vitest";
import type { IssueSummaryRecord, JobSummaryRecord } from "@metis/api";
import type { IssueTreeNode } from "../issues/useIssues";
import { TERMINAL_STATUSES } from "../../utils/statusMapping";
import { computeIssueProgress, computeIsActiveMap, countNeedsAttentionBadge } from "./computeIssueProgress";

// ---------------------------------------------------------------------------
// Test data helpers
// ---------------------------------------------------------------------------

function makeIssueRecord(
  overrides: Partial<{
    issue_id: string;
    status: string;
    assignee: string | null;
    description: string;
    creation_time: string;
  }> = {},
): IssueSummaryRecord {
  return {
    issue_id: overrides.issue_id ?? "i-default",
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    issue: {
      type: "task",
      title: "",
      description: overrides.description ?? "test issue",
      creator: "testuser",
      status: overrides.status ?? "open",
      assignee: overrides.assignee ?? null,
      progress: "",
      dependencies: [],
      patches: [],
    },
    creation_time: overrides.creation_time ?? "2026-01-01T00:00:00Z",
  } as IssueSummaryRecord;
}

function makeNode(
  id: string,
  overrides: Partial<{
    status: string;
    assignee: string | null;
    hardBlocked: boolean;
    children: IssueTreeNode[];
  }> = {},
): IssueTreeNode {
  return {
    id,
    issue: makeIssueRecord({
      issue_id: id,
      status: overrides.status ?? "open",
      assignee: overrides.assignee ?? null,
    }),
    children: overrides.children ?? [],
    defaultExpanded: false,
    blocked: false,
    blockedBy: [],
    hardBlocked: overrides.hardBlocked ?? false,
    hardBlockedBy: [],
  };
}

function makeJob(
  overrides: Partial<{
    job_id: string;
    status: string;
    start_time: string | null;
  }> = {},
): JobSummaryRecord {
  return {
    job_id: overrides.job_id ?? "t-default",
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    task: {
      prompt: "test job",
      creator: "testuser",
      status: overrides.status ?? "running",
      start_time: overrides.start_time ?? "2026-01-01T00:00:00Z",
    },
  } as JobSummaryRecord;
}

// ---------------------------------------------------------------------------
// computeIssueProgress tests
// ---------------------------------------------------------------------------

describe("computeIssueProgress", () => {
  it("returns empty array for empty input", () => {
    const result = computeIssueProgress([]);
    expect(result).toEqual([]);
  });

  it("returns zero counts for root with no children", () => {
    const root = makeNode("root-1");
    const result = computeIssueProgress([root]);
    expect(result).toHaveLength(1);
    expect(result[0].open).toBe(0);
    expect(result[0].inProgress).toBe(0);
    expect(result[0].closed).toBe(0);
    expect(result[0].total).toBe(0);
  });

  it("counts children with mixed statuses correctly", () => {
    const root = makeNode("root-1", {
      children: [
        makeNode("c1", { status: "open" }),
        makeNode("c2", { status: "in-progress" }),
        makeNode("c3", { status: "closed" }),
        makeNode("c4", { status: "open" }),
      ],
    });
    const result = computeIssueProgress([root]);
    expect(result[0].open).toBe(2);
    expect(result[0].inProgress).toBe(1);
    expect(result[0].closed).toBe(1);
  });

  it("skips hard-blocked children", () => {
    const root = makeNode("root-1", {
      children: [
        makeNode("c1", { status: "open" }),
        makeNode("c2", { status: "open", hardBlocked: true }),
        makeNode("c3", { status: "closed" }),
      ],
    });
    const result = computeIssueProgress([root]);
    expect(result[0].open).toBe(1);
    expect(result[0].closed).toBe(1);
    expect(result[0].total).toBe(2);
  });

  it("does not count failed, dropped, or rejected statuses", () => {
    const root = makeNode("root-1", {
      children: [
        makeNode("c1", { status: "failed" }),
        makeNode("c2", { status: "dropped" }),
        makeNode("c3", { status: "rejected" }),
        makeNode("c4", { status: "open" }),
      ],
    });
    const result = computeIssueProgress([root]);
    expect(result[0].open).toBe(1);
    expect(result[0].inProgress).toBe(0);
    expect(result[0].closed).toBe(0);
    expect(result[0].total).toBe(1);
  });

  it("produces independent entries for multiple roots", () => {
    const root1 = makeNode("root-1", {
      children: [
        makeNode("c1", { status: "open" }),
        makeNode("c2", { status: "closed" }),
      ],
    });
    const root2 = makeNode("root-2", {
      children: [
        makeNode("c3", { status: "in-progress" }),
      ],
    });
    const result = computeIssueProgress([root1, root2]);
    expect(result).toHaveLength(2);
    expect(result[0].rootId).toBe("root-1");
    expect(result[0].open).toBe(1);
    expect(result[0].closed).toBe(1);
    expect(result[1].rootId).toBe("root-2");
    expect(result[1].inProgress).toBe(1);
  });

  it("computes total as open + inProgress + closed", () => {
    const root = makeNode("root-1", {
      children: [
        makeNode("c1", { status: "open" }),
        makeNode("c2", { status: "in-progress" }),
        makeNode("c3", { status: "closed" }),
      ],
    });
    const result = computeIssueProgress([root]);
    expect(result[0].total).toBe(3);
    expect(result[0].total).toBe(
      result[0].open + result[0].inProgress + result[0].closed,
    );
  });

  it("preserves root metadata (rootId, rootIssue) in output", () => {
    const root = makeNode("root-1");
    const result = computeIssueProgress([root]);
    expect(result[0].rootId).toBe("root-1");
    expect(result[0].rootIssue).toBe(root.issue);
  });

  // ---------------------------------------------------------------------------
  // hasActive tests
  // ---------------------------------------------------------------------------

  it("hasActive is false when no sessionsByIssue provided", () => {
    const root = makeNode("root-1", {
      children: [makeNode("c1", { status: "open" })],
    });
    const result = computeIssueProgress([root]);
    expect(result[0].hasActive).toBe(false);
  });

  it("hasActive is false when sessionsByIssue is empty", () => {
    const root = makeNode("root-1", {
      children: [makeNode("c1", { status: "open" })],
    });
    const result = computeIssueProgress([root], new Map());
    expect(result[0].hasActive).toBe(false);
  });

  it("hasActive is true when root has a running job", () => {
    const root = makeNode("root-1");
    const sessionsByIssue = new Map([
      ["root-1", [makeJob({ status: "running" })]],
    ]);
    const result = computeIssueProgress([root], sessionsByIssue);
    expect(result[0].hasActive).toBe(true);
  });

  it("hasActive is true when a direct child has a pending job", () => {
    const child = makeNode("c1", { status: "open" });
    const root = makeNode("root-1", { children: [child] });
    const sessionsByIssue = new Map([
      ["c1", [makeJob({ status: "pending" })]],
    ]);
    const result = computeIssueProgress([root], sessionsByIssue);
    expect(result[0].hasActive).toBe(true);
  });

  it("hasActive is true when a grandchild has a running job (recursive)", () => {
    const grandchild = makeNode("gc1", { status: "open" });
    const child = makeNode("c1", { status: "open", children: [grandchild] });
    const root = makeNode("root-1", { children: [child] });
    const sessionsByIssue = new Map([
      ["gc1", [makeJob({ status: "running" })]],
    ]);
    const result = computeIssueProgress([root], sessionsByIssue);
    expect(result[0].hasActive).toBe(true);
  });

  it("hasActive is false when all jobs are complete/failed", () => {
    const root = makeNode("root-1", {
      children: [makeNode("c1", { status: "open" })],
    });
    const sessionsByIssue = new Map([
      ["root-1", [makeJob({ status: "complete" })]],
      ["c1", [makeJob({ status: "failed" })]],
    ]);
    const result = computeIssueProgress([root], sessionsByIssue);
    expect(result[0].hasActive).toBe(false);
  });

  it("hasActive is independent per root", () => {
    const root1 = makeNode("root-1", {
      children: [makeNode("c1", { status: "open" })],
    });
    const root2 = makeNode("root-2", {
      children: [makeNode("c2", { status: "open" })],
    });
    const sessionsByIssue = new Map([
      ["c1", [makeJob({ status: "running" })]],
    ]);
    const result = computeIssueProgress([root1, root2], sessionsByIssue);
    expect(result[0].hasActive).toBe(true);
    expect(result[1].hasActive).toBe(false);
  });

  // ---------------------------------------------------------------------------
  // needsAttentionCount tests
  // ---------------------------------------------------------------------------

  it("needsAttentionCount is 0 when no username provided", () => {
    const root = makeNode("root-1", {
      children: [makeNode("c1", { status: "open", assignee: "alice" })],
    });
    const result = computeIssueProgress([root]);
    expect(result[0].needsAttentionCount).toBe(0);
  });

  it("needsAttentionCount is 0 when no descendants match", () => {
    const root = makeNode("root-1", {
      children: [makeNode("c1", { status: "open", assignee: "bob" })],
    });
    const result = computeIssueProgress([root], undefined, "alice");
    expect(result[0].needsAttentionCount).toBe(0);
  });

  it("needsAttentionCount counts open descendants assigned to user with no active job", () => {
    const root = makeNode("root-1", {
      children: [
        makeNode("c1", { status: "open", assignee: "alice" }),
        makeNode("c2", { status: "open", assignee: "alice" }),
        makeNode("c3", { status: "open", assignee: "bob" }),
      ],
    });
    const result = computeIssueProgress([root], new Map(), "alice");
    expect(result[0].needsAttentionCount).toBe(2);
  });

  it("needsAttentionCount excludes descendants with running/pending jobs", () => {
    const root = makeNode("root-1", {
      children: [
        makeNode("c1", { status: "open", assignee: "alice" }),
        makeNode("c2", { status: "open", assignee: "alice" }),
      ],
    });
    const sessionsByIssue = new Map([
      ["c1", [makeJob({ status: "running" })]],
    ]);
    const result = computeIssueProgress([root], sessionsByIssue, "alice");
    expect(result[0].needsAttentionCount).toBe(1);
  });

  it("needsAttentionCount counts recursively through grandchildren", () => {
    const grandchild = makeNode("gc1", { status: "open", assignee: "alice" });
    const child = makeNode("c1", { status: "open", children: [grandchild] });
    const root = makeNode("root-1", { children: [child] });
    const result = computeIssueProgress([root], new Map(), "alice");
    // Both root (via countNeedsAttention on root itself) + child + grandchild can match.
    // root status is "open" but assignee is null, so doesn't match.
    // child status is "open" but assignee is null (default), so doesn't match.
    // grandchild status is "open" and assignee is "alice", so matches.
    expect(result[0].needsAttentionCount).toBe(1);
  });

  it("needsAttentionCount includes root itself if it matches", () => {
    const root = makeNode("root-1", { status: "open", assignee: "alice" });
    const result = computeIssueProgress([root], new Map(), "alice");
    expect(result[0].needsAttentionCount).toBe(1);
  });

  it("needsAttentionCount counts in-progress descendants assigned to user", () => {
    const root = makeNode("root-1", {
      children: [
        makeNode("c1", { status: "in-progress", assignee: "alice" }),
        makeNode("c2", { status: "open", assignee: "alice" }),
        makeNode("c3", { status: "closed", assignee: "alice" }),
      ],
    });
    const result = computeIssueProgress([root], new Map(), "alice");
    expect(result[0].needsAttentionCount).toBe(2); // c1 (in-progress) + c2 (open)
  });

  it("needsAttentionCount is independent per root", () => {
    const root1 = makeNode("root-1", {
      status: "open",
      assignee: "alice",
      children: [makeNode("c1", { status: "open", assignee: "alice" })],
    });
    const root2 = makeNode("root-2", {
      status: "open",
      assignee: "bob",
      children: [makeNode("c2", { status: "open", assignee: "bob" })],
    });
    const result = computeIssueProgress([root1, root2], new Map(), "alice");
    expect(result[0].needsAttentionCount).toBe(2); // root1 + c1
    expect(result[1].needsAttentionCount).toBe(0);
  });

  // ---------------------------------------------------------------------------
  // children hasActiveTask (recursive) tests
  // ---------------------------------------------------------------------------

  it("child hasActiveTask is true when child has a direct running job", () => {
    const root = makeNode("root-1", {
      children: [makeNode("c1", { status: "open" })],
    });
    const sessionsByIssue = new Map([["c1", [makeJob({ status: "running" })]]]);
    const result = computeIssueProgress([root], sessionsByIssue);
    expect(result[0].children[0].hasActiveTask).toBe(true);
  });

  it("child hasActiveTask is true when grandchild has a running job (recursive)", () => {
    const grandchild = makeNode("gc1", { status: "open" });
    const child = makeNode("c1", { status: "open", children: [grandchild] });
    const root = makeNode("root-1", { children: [child] });
    const sessionsByIssue = new Map([["gc1", [makeJob({ status: "running" })]]]);
    const result = computeIssueProgress([root], sessionsByIssue);
    expect(result[0].children[0].hasActiveTask).toBe(true);
  });

  it("child hasActiveTask is false when no descendant has running/pending job", () => {
    const grandchild = makeNode("gc1", { status: "open" });
    const child = makeNode("c1", { status: "open", children: [grandchild] });
    const root = makeNode("root-1", { children: [child] });
    const sessionsByIssue = new Map([["gc1", [makeJob({ status: "complete" })]]]);
    const result = computeIssueProgress([root], sessionsByIssue);
    expect(result[0].children[0].hasActiveTask).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// computeIsActiveMap tests
// ---------------------------------------------------------------------------

describe("computeIsActiveMap", () => {
  it("returns empty map for empty input", () => {
    const result = computeIsActiveMap([], new Map());
    expect(result.size).toBe(0);
  });

  it("marks issue as active when it has a running job", () => {
    const issues = [makeIssueRecord({ issue_id: "i-1" })];
    const sessionsByIssue = new Map([["i-1", [makeJob({ status: "running" })]]]);
    const result = computeIsActiveMap(issues, sessionsByIssue);
    expect(result.get("i-1")).toBe(true);
  });

  it("marks parent as active when child has running job", () => {
    const parent = makeIssueRecord({ issue_id: "parent" });
    const child = {
      ...makeIssueRecord({ issue_id: "child" }),
      issue: {
        ...makeIssueRecord({ issue_id: "child" }).issue,
        dependencies: [{ type: "child-of" as const, issue_id: "parent" }],
      },
    } as typeof parent;
    const sessionsByIssue = new Map([["child", [makeJob({ status: "running" })]]]);
    const result = computeIsActiveMap([parent, child], sessionsByIssue);
    expect(result.get("parent")).toBe(true);
    expect(result.get("child")).toBe(true);
  });

  it("marks grandparent as active when grandchild has running job", () => {
    const gp = makeIssueRecord({ issue_id: "gp" });
    const p = {
      ...makeIssueRecord({ issue_id: "p" }),
      issue: {
        ...makeIssueRecord({ issue_id: "p" }).issue,
        dependencies: [{ type: "child-of" as const, issue_id: "gp" }],
      },
    } as typeof gp;
    const c = {
      ...makeIssueRecord({ issue_id: "c" }),
      issue: {
        ...makeIssueRecord({ issue_id: "c" }).issue,
        dependencies: [{ type: "child-of" as const, issue_id: "p" }],
      },
    } as typeof gp;
    const sessionsByIssue = new Map([["c", [makeJob({ status: "pending" })]]]);
    const result = computeIsActiveMap([gp, p, c], sessionsByIssue);
    expect(result.get("gp")).toBe(true);
    expect(result.get("p")).toBe(true);
    expect(result.get("c")).toBe(true);
  });

  it("marks issue as inactive when all jobs are complete", () => {
    const issues = [makeIssueRecord({ issue_id: "i-1" })];
    const sessionsByIssue = new Map([["i-1", [makeJob({ status: "complete" })]]]);
    const result = computeIsActiveMap(issues, sessionsByIssue);
    expect(result.get("i-1")).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// Partitioning by TERMINAL_STATUSES (pattern used in IssueFilterSidebar)
// ---------------------------------------------------------------------------

describe("TERMINAL_STATUSES partitioning", () => {
  it("separates active roots from terminal roots", () => {
    const roots = [
      makeNode("r1", { status: "open" }),
      makeNode("r2", { status: "closed" }),
      makeNode("r3", { status: "in-progress" }),
      makeNode("r4", { status: "failed" }),
    ];
    const progress = computeIssueProgress(roots);
    const activeList = progress.filter(
      (p) => !TERMINAL_STATUSES.has(p.rootIssue.issue.status),
    );
    const completedList = progress.filter(
      (p) => TERMINAL_STATUSES.has(p.rootIssue.issue.status),
    );
    expect(activeList.map((p) => p.rootId)).toEqual(["r1", "r3"]);
    expect(completedList.map((p) => p.rootId)).toEqual(["r2", "r4"]);
  });

  it("returns empty completedList when no roots are terminal", () => {
    const roots = [
      makeNode("r1", { status: "open" }),
      makeNode("r2", { status: "in-progress" }),
    ];
    const progress = computeIssueProgress(roots);
    const completedList = progress.filter(
      (p) => TERMINAL_STATUSES.has(p.rootIssue.issue.status),
    );
    expect(completedList).toEqual([]);
  });

  it("returns empty activeList when all roots are terminal", () => {
    const roots = [
      makeNode("r1", { status: "closed" }),
      makeNode("r2", { status: "failed" }),
    ];
    const progress = computeIssueProgress(roots);
    const activeList = progress.filter(
      (p) => !TERMINAL_STATUSES.has(p.rootIssue.issue.status),
    );
    expect(activeList).toEqual([]);
  });

  it("blocked status is not terminal", () => {
    // "unknown" is not in TERMINAL_STATUSES, similar to how a blocked issue
    // would remain in the active list since its status stays as "open" or
    // "in-progress" rather than becoming a terminal status.
    const roots = [makeNode("r1", { status: "open" })];
    const progress = computeIssueProgress(roots);
    const activeList = progress.filter(
      (p) => !TERMINAL_STATUSES.has(p.rootIssue.issue.status),
    );
    expect(activeList).toHaveLength(1);
    expect(TERMINAL_STATUSES.has("open")).toBe(false);
    expect(TERMINAL_STATUSES.has("in-progress")).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// countNeedsAttentionBadge tests
// ---------------------------------------------------------------------------

describe("countNeedsAttentionBadge", () => {
  const assignedToAlice = (issue: IssueSummaryRecord) => issue.issue.assignee === "alice";

  it("returns 0 for empty issues list", () => {
    expect(countNeedsAttentionBadge([], assignedToAlice)).toBe(0);
  });

  it("counts issues matching filter with non-terminal status", () => {
    const issues = [
      makeIssueRecord({ issue_id: "i-1", status: "open", assignee: "alice" }),
      makeIssueRecord({ issue_id: "i-2", status: "in-progress", assignee: "alice" }),
      makeIssueRecord({ issue_id: "i-3", status: "blocked", assignee: "alice" }),
      makeIssueRecord({ issue_id: "i-4", status: "closed", assignee: "alice" }),
    ];
    expect(countNeedsAttentionBadge(issues, assignedToAlice)).toBe(3);
  });

  it("does not count issues that do not match filter", () => {
    const issues = [
      makeIssueRecord({ issue_id: "i-1", status: "open", assignee: "bob" }),
      makeIssueRecord({ issue_id: "i-2", status: "open", assignee: "alice" }),
    ];
    expect(countNeedsAttentionBadge(issues, assignedToAlice)).toBe(1);
  });

  it("does not count issues with null assignee", () => {
    const issues = [
      makeIssueRecord({ issue_id: "i-1", status: "open", assignee: null }),
    ];
    expect(countNeedsAttentionBadge(issues, assignedToAlice)).toBe(0);
  });

  it("excludes issues with active jobs when isActiveMap is provided", () => {
    const issues = [
      makeIssueRecord({ issue_id: "i-1", status: "open", assignee: "alice" }),
      makeIssueRecord({ issue_id: "i-2", status: "open", assignee: "alice" }),
    ];
    const isActiveMap = new Map([["i-1", true], ["i-2", false]]);
    expect(countNeedsAttentionBadge(issues, assignedToAlice, isActiveMap)).toBe(1);
  });

  it("counts all matching issues when isActiveMap is not provided", () => {
    const issues = [
      makeIssueRecord({ issue_id: "i-1", status: "open", assignee: "alice" }),
      makeIssueRecord({ issue_id: "i-2", status: "open", assignee: "alice" }),
    ];
    expect(countNeedsAttentionBadge(issues, assignedToAlice)).toBe(2);
  });

  it("excludes terminal statuses like failed and closed", () => {
    const issues = [
      makeIssueRecord({ issue_id: "i-1", status: "failed", assignee: "alice" }),
      makeIssueRecord({ issue_id: "i-2", status: "closed", assignee: "alice" }),
      makeIssueRecord({ issue_id: "i-3", status: "open", assignee: "alice" }),
    ];
    expect(countNeedsAttentionBadge(issues, assignedToAlice)).toBe(1);
  });

  it("excludes rejected issues from attention count", () => {
    const issues = [
      makeIssueRecord({ issue_id: "i-1", status: "rejected", assignee: "alice" }),
      makeIssueRecord({ issue_id: "i-2", status: "open", assignee: "alice" }),
      makeIssueRecord({ issue_id: "i-3", status: "in-progress", assignee: "alice" }),
    ];
    expect(countNeedsAttentionBadge(issues, assignedToAlice)).toBe(2);
  });
});
