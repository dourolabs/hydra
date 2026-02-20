import type { IssueVersionRecord, IssueStatus } from "@metis/api";

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
 *   it is conservatively treated as blocking.
 * - hardBlocked: true when the issue has a "blocked-on" dependency on an issue
 *   whose status is "failed", "rejected", or "dropped". Hard-blocked is always
 *   a subset of blocked.
 */
export function computeBlockedStatus(
  record: IssueVersionRecord,
  issueMap: Map<string, IssueVersionRecord>,
): BlockedStatus {
  const blockedBy: string[] = [];
  const hardBlockedBy: string[] = [];

  for (const dep of record.issue.dependencies) {
    if (dep.type !== "blocked-on") continue;

    const target = issueMap.get(dep.issue_id);

    if (!target) {
      // Missing target — conservatively treat as blocking (but not hard-blocked)
      blockedBy.push(dep.issue_id);
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
