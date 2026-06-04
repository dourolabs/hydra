import type { BadgeStatus } from "@hydra/ui";

/** Normalize a PascalCase patch status (e.g. "ChangesRequested") to a BadgeStatus ("changes-requested"). */
export function normalizePatchStatus(status: string): BadgeStatus {
  const map: Record<string, BadgeStatus> = {
    Open: "open",
    Merged: "merged",
    Closed: "closed",
    ChangesRequested: "changes-requested",
  };
  return map[status] ?? "unknown";
}

/** Normalize a PascalCase CI state (e.g. "Success") to a BadgeStatus ("success"). */
export function normalizeCiState(state: string): BadgeStatus {
  const map: Record<string, BadgeStatus> = {
    Success: "success",
    Failed: "failed",
    Pending: "pending",
  };
  return map[state] ?? "unknown";
}

/** Normalize a lowercase session status to a BadgeStatus. Session statuses already match 1:1. */
export function normalizeSessionStatus(status: string): BadgeStatus {
  const valid: Set<string> = new Set([
    "created",
    "pending",
    "running",
    "complete",
    "failed",
  ]);
  if (valid.has(status)) return status as BadgeStatus;
  return "unknown";
}
