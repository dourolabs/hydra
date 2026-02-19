import type { BadgeStatus } from "@metis/ui";

const validStatuses: Set<string> = new Set([
  "open",
  "in-progress",
  "closed",
  "failed",
  "dropped",
  "blocked",
  "rejected",
]);

/** Map an issue status string to a BadgeStatus. */
export function issueToBadgeStatus(status: string): BadgeStatus {
  if (validStatuses.has(status)) return status as BadgeStatus;
  return "open";
}

/** Map a job status string to a BadgeStatus. */
export function jobToBadgeStatus(status: string): BadgeStatus {
  const mapped: Record<string, BadgeStatus> = {
    created: "open",
    pending: "open",
    running: "in-progress",
    complete: "closed",
    failed: "failed",
  };
  const s = mapped[status];
  if (s) return s;
  if (validStatuses.has(status)) return status as BadgeStatus;
  return "open";
}

/** Map a patch status string to a BadgeStatus. */
export function patchToBadgeStatus(status: string): BadgeStatus {
  const mapped: Record<string, BadgeStatus> = {
    Open: "open",
    Merged: "closed",
    Closed: "failed",
    ChangesRequested: "rejected",
  };
  const s = mapped[status];
  return s ?? "open";
}
