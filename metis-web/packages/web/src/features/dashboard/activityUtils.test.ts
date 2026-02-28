import { describe, it, expect } from "vitest";
import type { IssueSummaryRecord, JobSummaryRecord } from "@metis/api";
import type { IssueTreeNode } from "../issues/useIssues";
import {
  classifyActivity,
  collectActivityItems,
  sortActivityItems,
  computeSummary,
  sectionLabel,
  computeIssueProgress,
  type ActivityItem,
  type ActivitySection,
} from "./activityUtils";

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

function makeIssueRecord(
  overrides: {
    issue_id?: string;
    version?: bigint;
    timestamp?: string;
    creation_time?: string;
    status?: string;
    assignee?: string;
    description?: string;
  } = {},
): IssueSummaryRecord {
  return {
    issue_id: overrides.issue_id ?? "issue-1",
    version: overrides.version ?? BigInt(1),
    timestamp: overrides.timestamp ?? "2026-01-01T00:00:00Z",
    creation_time: overrides.creation_time ?? "2026-01-01T00:00:00Z",
    issue: {
      type: "task",
      description: overrides.description ?? "Test issue",
      creator: "testuser",
      status: (overrides.status ?? "open") as IssueSummaryRecord["issue"]["status"],
      assignee: overrides.assignee,
      dependencies: [],
      patches: [],
    },
  };
}

function makeNode(
  overrides: {
    id?: string;
    status?: string;
    assignee?: string;
    description?: string;
    timestamp?: string;
    children?: IssueTreeNode[];
    hardBlocked?: boolean;
  } = {},
): IssueTreeNode {
  return {
    id: overrides.id ?? "node-1",
    issue: makeIssueRecord({
      status: overrides.status,
      assignee: overrides.assignee,
      description: overrides.description,
      timestamp: overrides.timestamp,
    }),
    children: overrides.children ?? [],
    defaultExpanded: true,
    blocked: false,
    blockedBy: [],
    hardBlocked: overrides.hardBlocked ?? false,
    hardBlockedBy: [],
  };
}

function makeJob(
  overrides: {
    job_id?: string;
    status?: string;
    start_time?: string;
  } = {},
): JobSummaryRecord {
  return {
    job_id: overrides.job_id ?? "job-1",
    version: BigInt(1),
    timestamp: "2026-01-01T00:00:00Z",
    task: {
      prompt: "test prompt",
      creator: "testuser",
      status: (overrides.status ?? "running") as JobSummaryRecord["task"]["status"],
      start_time: overrides.start_time ?? "2026-01-01T01:00:00Z",
    },
  };
}

// ---------------------------------------------------------------------------
// classifyActivity
// ---------------------------------------------------------------------------

describe("classifyActivity", () => {
  const emptyJobs = new Map<string, JobSummaryRecord[]>();

  it("returns 'active' for a node with a running job", () => {
    const node = makeNode({ id: "n1" });
    const jobs = new Map([["n1", [makeJob({ status: "running" })]]]);
    expect(classifyActivity(node, jobs, "me")).toBe("active");
  });

  it("returns 'active' for a node with a pending job", () => {
    const node = makeNode({ id: "n1" });
    const jobs = new Map([["n1", [makeJob({ status: "pending" })]]]);
    expect(classifyActivity(node, jobs, "me")).toBe("active");
  });

  it("returns 'recently-completed' for terminal status 'closed' with no active jobs", () => {
    const node = makeNode({ id: "n1", status: "closed" });
    expect(classifyActivity(node, emptyJobs, "me")).toBe("recently-completed");
  });

  it("returns 'recently-completed' for terminal status 'failed'", () => {
    const node = makeNode({ id: "n1", status: "failed" });
    expect(classifyActivity(node, emptyJobs, "me")).toBe("recently-completed");
  });

  it("returns 'recently-completed' for terminal status 'rejected'", () => {
    const node = makeNode({ id: "n1", status: "rejected" });
    expect(classifyActivity(node, emptyJobs, "me")).toBe("recently-completed");
  });

  it("returns 'recently-completed' for terminal status 'dropped'", () => {
    const node = makeNode({ id: "n1", status: "dropped" });
    expect(classifyActivity(node, emptyJobs, "me")).toBe("recently-completed");
  });

  it("returns 'needs-attention' for open node assigned to current user", () => {
    const node = makeNode({ id: "n1", status: "open", assignee: "me" });
    expect(classifyActivity(node, emptyJobs, "me")).toBe("needs-attention");
  });

  it("returns 'upcoming' for open node assigned to a different user", () => {
    const node = makeNode({
      id: "n1",
      status: "open",
      assignee: "someone-else",
    });
    expect(classifyActivity(node, emptyJobs, "me")).toBe("upcoming");
  });

  it("returns 'upcoming' for open node with no assignee", () => {
    const node = makeNode({ id: "n1", status: "open" });
    expect(classifyActivity(node, emptyJobs, "me")).toBe("upcoming");
  });

  it("returns 'upcoming' for in-progress node with no active jobs", () => {
    const node = makeNode({ id: "n1", status: "in-progress" });
    expect(classifyActivity(node, emptyJobs, "me")).toBe("upcoming");
  });

  it("does not return 'active' when jobs are completed/failed (not running/pending)", () => {
    const node = makeNode({ id: "n1" });
    const jobs = new Map([
      [
        "n1",
        [
          makeJob({ status: "complete" }),
          makeJob({ status: "failed" }),
        ],
      ],
    ]);
    expect(classifyActivity(node, jobs, "me")).not.toBe("active");
  });
});

// ---------------------------------------------------------------------------
// collectActivityItems
// ---------------------------------------------------------------------------

describe("collectActivityItems", () => {
  const emptyJobs = new Map<string, JobSummaryRecord[]>();

  it("returns empty array for empty roots", () => {
    expect(collectActivityItems([], emptyJobs, "me")).toEqual([]);
  });

  it("includes root itself when it has no children", () => {
    const root = makeNode({ id: "root-1" });
    const items = collectActivityItems([root], emptyJobs, "me");
    expect(items).toHaveLength(1);
    expect(items[0].issueId).toBe("root-1");
  });

  it("includes children but NOT the root when root has children", () => {
    const child1 = makeNode({ id: "child-1" });
    const child2 = makeNode({ id: "child-2" });
    const root = makeNode({ id: "root-1", children: [child1, child2] });
    const items = collectActivityItems([root], emptyJobs, "me");
    const ids = items.map((i) => i.issueId);
    expect(ids).toContain("child-1");
    expect(ids).toContain("child-2");
    expect(ids).not.toContain("root-1");
  });

  it("recursively includes grandchildren", () => {
    const grandchild = makeNode({ id: "gc-1" });
    const child = makeNode({ id: "child-1", children: [grandchild] });
    const root = makeNode({ id: "root-1", children: [child] });
    const items = collectActivityItems([root], emptyJobs, "me");
    const ids = items.map((i) => i.issueId);
    expect(ids).toContain("child-1");
    expect(ids).toContain("gc-1");
  });

  it("skips hard-blocked nodes", () => {
    const blocked = makeNode({ id: "blocked-1", hardBlocked: true });
    const root = makeNode({ id: "root-1", children: [blocked] });
    const items = collectActivityItems([root], emptyJobs, "me");
    expect(items.map((i) => i.issueId)).not.toContain("blocked-1");
  });

  it("hard-blocked child does not prevent siblings from being collected", () => {
    const blocked = makeNode({ id: "blocked-1", hardBlocked: true });
    const sibling = makeNode({ id: "sibling-1" });
    const root = makeNode({
      id: "root-1",
      children: [blocked, sibling],
    });
    const items = collectActivityItems([root], emptyJobs, "me");
    const ids = items.map((i) => i.issueId);
    expect(ids).not.toContain("blocked-1");
    expect(ids).toContain("sibling-1");
  });

  it("propagates correct parentIssueId and parentDescription", () => {
    const child = makeNode({ id: "child-1" });
    const root = makeNode({
      id: "root-1",
      description: "Root description\nSecond line",
      children: [child],
    });
    const items = collectActivityItems([root], emptyJobs, "me");
    expect(items[0].parentIssueId).toBe("root-1");
    expect(items[0].parentDescription).toBe("Root description");
  });

  it("sets activeJob for running/pending jobs and omits it otherwise", () => {
    const child = makeNode({ id: "child-1" });
    const childNoJob = makeNode({ id: "child-2" });
    const root = makeNode({
      id: "root-1",
      children: [child, childNoJob],
    });
    const runningJob = makeJob({
      job_id: "j1",
      status: "running",
      start_time: "2026-01-02T00:00:00Z",
    });
    const jobs = new Map([["child-1", [runningJob]]]);
    const items = collectActivityItems([root], jobs, "me");
    const withJob = items.find((i) => i.issueId === "child-1");
    const withoutJob = items.find((i) => i.issueId === "child-2");
    expect(withJob?.activeJob).toBeDefined();
    expect(withJob?.activeJob?.job_id).toBe("j1");
    expect(withoutJob?.activeJob).toBeUndefined();
  });

  it("uses activeJob.start_time for sortTime when job is active, falls back to issue timestamp", () => {
    const child = makeNode({
      id: "child-1",
      timestamp: "2026-01-01T00:00:00Z",
    });
    const childNoJob = makeNode({
      id: "child-2",
      timestamp: "2026-02-01T00:00:00Z",
    });
    const root = makeNode({
      id: "root-1",
      children: [child, childNoJob],
    });
    const job = makeJob({
      status: "running",
      start_time: "2026-03-01T00:00:00Z",
    });
    const jobs = new Map([["child-1", [job]]]);
    const items = collectActivityItems([root], jobs, "me");
    const active = items.find((i) => i.issueId === "child-1");
    const inactive = items.find((i) => i.issueId === "child-2");
    expect(active?.sortTime).toBe("2026-03-01T00:00:00Z");
    expect(inactive?.sortTime).toBe("2026-02-01T00:00:00Z");
  });

  it("collects items from multiple roots", () => {
    const root1 = makeNode({ id: "root-1" });
    const root2 = makeNode({ id: "root-2" });
    const items = collectActivityItems([root1, root2], emptyJobs, "me");
    expect(items).toHaveLength(2);
    const ids = items.map((i) => i.issueId);
    expect(ids).toContain("root-1");
    expect(ids).toContain("root-2");
  });
});

// ---------------------------------------------------------------------------
// sortActivityItems
// ---------------------------------------------------------------------------

describe("sortActivityItems", () => {
  function makeItem(
    section: ActivitySection,
    sortTime: string,
  ): ActivityItem {
    return {
      issueId: `${section}-${sortTime}`,
      issue: makeIssueRecord(),
      parentIssueId: "parent",
      parentDescription: "Parent",
      section,
      sortTime,
    };
  }

  it("returns empty array for empty input", () => {
    expect(sortActivityItems([])).toEqual([]);
  });

  it("sorts items in correct section order: active < needs-attention < upcoming < recently-completed", () => {
    const completed = makeItem("recently-completed", "2026-01-01T00:00:00Z");
    const upcoming = makeItem("upcoming", "2026-01-01T00:00:00Z");
    const needsAttention = makeItem("needs-attention", "2026-01-01T00:00:00Z");
    const active = makeItem("active", "2026-01-01T00:00:00Z");

    const sorted = sortActivityItems([
      completed,
      upcoming,
      needsAttention,
      active,
    ]);
    expect(sorted.map((i) => i.section)).toEqual([
      "active",
      "needs-attention",
      "upcoming",
      "recently-completed",
    ]);
  });

  it("sorts by most recent sortTime first within the same section", () => {
    const older = makeItem("active", "2026-01-01T00:00:00Z");
    const newer = makeItem("active", "2026-02-01T00:00:00Z");

    const sorted = sortActivityItems([older, newer]);
    expect(sorted[0].sortTime).toBe("2026-02-01T00:00:00Z");
    expect(sorted[1].sortTime).toBe("2026-01-01T00:00:00Z");
  });

  it("produces correct order with mixed sections and timestamps", () => {
    const items = [
      makeItem("upcoming", "2026-01-01T00:00:00Z"),
      makeItem("active", "2026-01-01T00:00:00Z"),
      makeItem("active", "2026-02-01T00:00:00Z"),
      makeItem("recently-completed", "2026-03-01T00:00:00Z"),
    ];

    const sorted = sortActivityItems(items);
    expect(sorted.map((i) => i.section)).toEqual([
      "active",
      "active",
      "upcoming",
      "recently-completed",
    ]);
    // Within active section, newer first
    expect(sorted[0].sortTime).toBe("2026-02-01T00:00:00Z");
    expect(sorted[1].sortTime).toBe("2026-01-01T00:00:00Z");
  });

  it("returns a new array (does not mutate input)", () => {
    const items = [
      makeItem("upcoming", "2026-01-01T00:00:00Z"),
      makeItem("active", "2026-01-01T00:00:00Z"),
    ];
    const original = [...items];
    const sorted = sortActivityItems(items);
    expect(sorted).not.toBe(items);
    expect(items).toEqual(original);
  });
});

// ---------------------------------------------------------------------------
// computeSummary
// ---------------------------------------------------------------------------

describe("computeSummary", () => {
  function makeItem(section: ActivitySection): ActivityItem {
    return {
      issueId: `item-${section}`,
      issue: makeIssueRecord(),
      parentIssueId: "parent",
      parentDescription: "Parent",
      section,
      sortTime: "2026-01-01T00:00:00Z",
    };
  }

  it("returns all counts zero for empty items", () => {
    const summary = computeSummary([]);
    expect(summary).toEqual({
      activeCount: 0,
      needsAttentionCount: 0,
      completedCount: 0,
      totalCount: 0,
    });
  });

  it("counts only active items correctly", () => {
    const items = [makeItem("active"), makeItem("active")];
    const summary = computeSummary(items);
    expect(summary.activeCount).toBe(2);
    expect(summary.needsAttentionCount).toBe(0);
    expect(summary.completedCount).toBe(0);
    expect(summary.totalCount).toBe(2);
  });

  it("counts only needs-attention items correctly", () => {
    const items = [makeItem("needs-attention"), makeItem("needs-attention")];
    const summary = computeSummary(items);
    expect(summary.needsAttentionCount).toBe(2);
    expect(summary.activeCount).toBe(0);
    expect(summary.completedCount).toBe(0);
    expect(summary.totalCount).toBe(2);
  });

  it("counts only recently-completed items correctly", () => {
    const items = [
      makeItem("recently-completed"),
      makeItem("recently-completed"),
      makeItem("recently-completed"),
    ];
    const summary = computeSummary(items);
    expect(summary.completedCount).toBe(3);
    expect(summary.activeCount).toBe(0);
    expect(summary.needsAttentionCount).toBe(0);
    expect(summary.totalCount).toBe(3);
  });

  it("counts upcoming items toward totalCount but not other counts", () => {
    const items = [makeItem("upcoming"), makeItem("upcoming")];
    const summary = computeSummary(items);
    expect(summary.activeCount).toBe(0);
    expect(summary.needsAttentionCount).toBe(0);
    expect(summary.completedCount).toBe(0);
    expect(summary.totalCount).toBe(2);
  });

  it("counts mixed items correctly", () => {
    const items = [
      makeItem("active"),
      makeItem("active"),
      makeItem("needs-attention"),
      makeItem("upcoming"),
      makeItem("recently-completed"),
      makeItem("recently-completed"),
    ];
    const summary = computeSummary(items);
    expect(summary.activeCount).toBe(2);
    expect(summary.needsAttentionCount).toBe(1);
    expect(summary.completedCount).toBe(2);
    expect(summary.totalCount).toBe(6);
  });
});

// ---------------------------------------------------------------------------
// sectionLabel
// ---------------------------------------------------------------------------

describe("sectionLabel", () => {
  it('returns "ACTIVE" for "active"', () => {
    expect(sectionLabel("active")).toBe("ACTIVE");
  });

  it('returns "NEEDS ATTENTION" for "needs-attention"', () => {
    expect(sectionLabel("needs-attention")).toBe("NEEDS ATTENTION");
  });

  it('returns "UPCOMING" for "upcoming"', () => {
    expect(sectionLabel("upcoming")).toBe("UPCOMING");
  });

  it('returns "COMPLETED" for "recently-completed"', () => {
    expect(sectionLabel("recently-completed")).toBe("COMPLETED");
  });
});

// ---------------------------------------------------------------------------
// computeIssueProgress
// ---------------------------------------------------------------------------

describe("computeIssueProgress", () => {
  it("returns an empty array for empty input", () => {
    expect(computeIssueProgress([])).toEqual([]);
  });

  it("returns zero counts for a root with no children", () => {
    const root = makeNode({ id: "root", status: "open" });
    const result = computeIssueProgress([root]);

    expect(result).toHaveLength(1);
    expect(result[0].open).toBe(0);
    expect(result[0].inProgress).toBe(0);
    expect(result[0].closed).toBe(0);
    expect(result[0].total).toBe(0);
  });

  it("counts children with mixed statuses correctly", () => {
    const root = makeNode({
      id: "root",
      status: "open",
      children: [
        makeNode({ id: "c1", status: "open" }),
        makeNode({ id: "c2", status: "in-progress" }),
        makeNode({ id: "c3", status: "closed" }),
        makeNode({ id: "c4", status: "open" }),
        makeNode({ id: "c5", status: "closed" }),
      ],
    });
    const result = computeIssueProgress([root]);

    expect(result).toHaveLength(1);
    expect(result[0].open).toBe(2);
    expect(result[0].inProgress).toBe(1);
    expect(result[0].closed).toBe(2);
  });

  it("skips hard-blocked children", () => {
    const root = makeNode({
      id: "root",
      status: "open",
      children: [
        makeNode({ id: "c1", status: "open" }),
        makeNode({ id: "c2", status: "open", hardBlocked: true }),
        makeNode({ id: "c3", status: "closed" }),
      ],
    });
    const result = computeIssueProgress([root]);

    expect(result).toHaveLength(1);
    expect(result[0].open).toBe(1);
    expect(result[0].closed).toBe(1);
    expect(result[0].total).toBe(2);
  });

  it("does not count failed, dropped, or rejected statuses", () => {
    const root = makeNode({
      id: "root",
      status: "open",
      children: [
        makeNode({ id: "c1", status: "open" }),
        makeNode({ id: "c2", status: "failed" }),
        makeNode({ id: "c3", status: "dropped" }),
        makeNode({ id: "c4", status: "rejected" }),
        makeNode({ id: "c5", status: "closed" }),
      ],
    });
    const result = computeIssueProgress([root]);

    expect(result[0].open).toBe(1);
    expect(result[0].inProgress).toBe(0);
    expect(result[0].closed).toBe(1);
    expect(result[0].total).toBe(2);
  });

  it("produces independent entries for multiple roots", () => {
    const root1 = makeNode({
      id: "r1",
      status: "open",
      children: [
        makeNode({ id: "c1", status: "open" }),
        makeNode({ id: "c2", status: "closed" }),
      ],
    });
    const root2 = makeNode({
      id: "r2",
      status: "open",
      children: [
        makeNode({ id: "c3", status: "in-progress" }),
      ],
    });
    const result = computeIssueProgress([root1, root2]);

    expect(result).toHaveLength(2);
    expect(result[0].rootId).toBe("r1");
    expect(result[0].open).toBe(1);
    expect(result[0].closed).toBe(1);
    expect(result[1].rootId).toBe("r2");
    expect(result[1].inProgress).toBe(1);
  });

  it("computes total as open + inProgress + closed", () => {
    const root = makeNode({
      id: "root",
      status: "open",
      children: [
        makeNode({ id: "c1", status: "open" }),
        makeNode({ id: "c2", status: "in-progress" }),
        makeNode({ id: "c3", status: "closed" }),
        makeNode({ id: "c4", status: "failed" }),
        makeNode({ id: "c5", status: "open", hardBlocked: true }),
      ],
    });
    const result = computeIssueProgress([root]);

    expect(result[0].total).toBe(
      result[0].open + result[0].inProgress + result[0].closed,
    );
    expect(result[0].total).toBe(3);
  });

  it("preserves root metadata in the output", () => {
    const root = makeNode({
      id: "my-root",
      status: "in-progress",
      children: [
        makeNode({ id: "c1", status: "open" }),
      ],
    });
    const result = computeIssueProgress([root]);

    expect(result[0].rootId).toBe("my-root");
    expect(result[0].rootIssue).toBe(root.issue);
  });
});
