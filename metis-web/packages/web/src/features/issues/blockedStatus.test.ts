import { describe, it, expect } from "vitest";
import type { IssueSummaryRecord } from "@metis/api";
import { computeBlockedStatus } from "./blockedStatus";

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

function buildMap(
  records: IssueSummaryRecord[],
): Map<string, IssueSummaryRecord> {
  const map = new Map<string, IssueSummaryRecord>();
  for (const r of records) {
    map.set(r.issue_id, r);
  }
  return map;
}

describe("computeBlockedStatus", () => {
  it("returns not blocked when there are no dependencies", () => {
    const record = makeRecord("a", "open");
    const issueMap = buildMap([record]);

    const result = computeBlockedStatus(record, issueMap);

    expect(result.blocked).toBe(false);
    expect(result.blockedBy).toEqual([]);
    expect(result.hardBlocked).toBe(false);
    expect(result.hardBlockedBy).toEqual([]);
  });

  it("returns not blocked when only child-of dependencies exist", () => {
    const parent = makeRecord("parent", "open");
    const child = makeRecord("child", "open", [
      { type: "child-of", issue_id: "parent" },
    ]);
    const issueMap = buildMap([parent, child]);

    const result = computeBlockedStatus(child, issueMap);

    expect(result.blocked).toBe(false);
    expect(result.blockedBy).toEqual([]);
  });

  it("returns not blocked when blocked-on target is closed", () => {
    const blocker = makeRecord("blocker", "closed");
    const issue = makeRecord("issue", "open", [
      { type: "blocked-on", issue_id: "blocker" },
    ]);
    const issueMap = buildMap([blocker, issue]);

    const result = computeBlockedStatus(issue, issueMap);

    expect(result.blocked).toBe(false);
    expect(result.blockedBy).toEqual([]);
    expect(result.hardBlocked).toBe(false);
    expect(result.hardBlockedBy).toEqual([]);
  });

  it("returns blocked when blocked-on target is open", () => {
    const blocker = makeRecord("blocker", "open");
    const issue = makeRecord("issue", "open", [
      { type: "blocked-on", issue_id: "blocker" },
    ]);
    const issueMap = buildMap([blocker, issue]);

    const result = computeBlockedStatus(issue, issueMap);

    expect(result.blocked).toBe(true);
    expect(result.blockedBy).toEqual(["blocker"]);
    expect(result.hardBlocked).toBe(false);
    expect(result.hardBlockedBy).toEqual([]);
  });

  it("returns blocked when blocked-on target is in-progress", () => {
    const blocker = makeRecord("blocker", "in-progress");
    const issue = makeRecord("issue", "open", [
      { type: "blocked-on", issue_id: "blocker" },
    ]);
    const issueMap = buildMap([blocker, issue]);

    const result = computeBlockedStatus(issue, issueMap);

    expect(result.blocked).toBe(true);
    expect(result.blockedBy).toEqual(["blocker"]);
    expect(result.hardBlocked).toBe(false);
    expect(result.hardBlockedBy).toEqual([]);
  });

  it("returns hard-blocked when blocked-on target is failed", () => {
    const blocker = makeRecord("blocker", "failed");
    const issue = makeRecord("issue", "open", [
      { type: "blocked-on", issue_id: "blocker" },
    ]);
    const issueMap = buildMap([blocker, issue]);

    const result = computeBlockedStatus(issue, issueMap);

    expect(result.blocked).toBe(true);
    expect(result.blockedBy).toEqual(["blocker"]);
    expect(result.hardBlocked).toBe(true);
    expect(result.hardBlockedBy).toEqual(["blocker"]);
  });

  it("returns hard-blocked when blocked-on target is rejected", () => {
    const blocker = makeRecord("blocker", "rejected");
    const issue = makeRecord("issue", "open", [
      { type: "blocked-on", issue_id: "blocker" },
    ]);
    const issueMap = buildMap([blocker, issue]);

    const result = computeBlockedStatus(issue, issueMap);

    expect(result.blocked).toBe(true);
    expect(result.blockedBy).toEqual(["blocker"]);
    expect(result.hardBlocked).toBe(true);
    expect(result.hardBlockedBy).toEqual(["blocker"]);
  });

  it("returns hard-blocked when blocked-on target is dropped", () => {
    const blocker = makeRecord("blocker", "dropped");
    const issue = makeRecord("issue", "open", [
      { type: "blocked-on", issue_id: "blocker" },
    ]);
    const issueMap = buildMap([blocker, issue]);

    const result = computeBlockedStatus(issue, issueMap);

    expect(result.blocked).toBe(true);
    expect(result.blockedBy).toEqual(["blocker"]);
    expect(result.hardBlocked).toBe(true);
    expect(result.hardBlockedBy).toEqual(["blocker"]);
  });

  it("handles multiple blockers with mixed statuses", () => {
    const closedIssue = makeRecord("closed-one", "closed");
    const openIssue = makeRecord("open-one", "open");
    const failedIssue = makeRecord("failed-one", "failed");
    const issue = makeRecord("issue", "open", [
      { type: "blocked-on", issue_id: "closed-one" },
      { type: "blocked-on", issue_id: "open-one" },
      { type: "blocked-on", issue_id: "failed-one" },
    ]);
    const issueMap = buildMap([closedIssue, openIssue, failedIssue, issue]);

    const result = computeBlockedStatus(issue, issueMap);

    expect(result.blocked).toBe(true);
    expect(result.blockedBy).toEqual(["open-one", "failed-one"]);
    expect(result.hardBlocked).toBe(true);
    expect(result.hardBlockedBy).toEqual(["failed-one"]);
  });

  it("treats missing blocker issues as not blocking", () => {
    const issue = makeRecord("issue", "open", [
      { type: "blocked-on", issue_id: "nonexistent" },
    ]);
    const issueMap = buildMap([issue]);

    const result = computeBlockedStatus(issue, issueMap);

    expect(result.blocked).toBe(false);
    expect(result.blockedBy).toEqual([]);
    expect(result.hardBlocked).toBe(false);
    expect(result.hardBlockedBy).toEqual([]);
  });

  it("ignores child-of dependencies in blocked computation", () => {
    const parent = makeRecord("parent", "open");
    const issue = makeRecord("issue", "open", [
      { type: "child-of", issue_id: "parent" },
      { type: "blocked-on", issue_id: "parent" },
    ]);
    const issueMap = buildMap([parent, issue]);

    const result = computeBlockedStatus(issue, issueMap);

    expect(result.blocked).toBe(true);
    expect(result.blockedBy).toEqual(["parent"]);
  });
});
