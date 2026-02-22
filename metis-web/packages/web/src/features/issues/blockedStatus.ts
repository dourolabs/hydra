import type { IssueSummaryRecord, IssueStatus } from "@metis/api";

export interface BlockedStatus {
  blocked: boolean;
  blockedBy: string[];
  hardBlocked: boolean;
  hardBlockedBy: string[];
}

const HARD_BLOCKED_STATUSES: ReadonlySet<IssueStatus> = new Set([
  "failed",
  "rejected",
  "dropped",
]);

/**
 * Compute blocked and hard-blocked status for an issue based on its
 * "blocked-on" dependencies and the current state of those dependency targets.
 *
 * - blocked: true when the issue has a "blocked-on" dependency on an issue
 *   whose status is NOT "closed". If the target issue is missing from the map,
 *   it is treated as not blocking (skipped).
 * - hardBlocked: true when the issue has a "blocked-on" dependency on an issue
 *   whose status is "failed", "rejected", or "dropped". Hard-blocked is always
 *   a subset of blocked.
 */
export function computeBlockedStatus(
  record: IssueSummaryRecord,
  issueMap: Map<string, IssueSummaryRecord>,
): BlockedStatus {
  const blockedBy: string[] = [];
  const hardBlockedBy: string[] = [];

  for (const dep of record.issue.dependencies) {
    if (dep.type !== "blocked-on") continue;

    const target = issueMap.get(dep.issue_id);

    if (!target) {
      // Missing target — treat as not blocking (skip)
      continue;
    }

    if (target.issue.status === "closed") continue;

    blockedBy.push(dep.issue_id);

    if (HARD_BLOCKED_STATUSES.has(target.issue.status)) {
      hardBlockedBy.push(dep.issue_id);
    }
  }

  return {
    blocked: blockedBy.length > 0,
    blockedBy,
    hardBlocked: hardBlockedBy.length > 0,
    hardBlockedBy,
  };
}
