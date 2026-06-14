import type { BadgeStatus } from "@hydra/ui";

/** Validate a patch status wire value and return the matching BadgeStatus. */
export function normalizePatchStatus(status: string): BadgeStatus {
  const valid = new Set(["open", "merged", "closed", "changes-requested"]);
  return valid.has(status) ? (status as BadgeStatus) : "unknown";
}

/** Validate a CI state wire value and return the matching BadgeStatus. */
export function normalizeCiState(state: string): BadgeStatus {
  const valid = new Set(["success", "failed", "pending"]);
  return valid.has(state) ? (state as BadgeStatus) : "unknown";
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
