import type { SessionSummaryRecord, Status } from "@hydra/api";

const ACTIVE_STATUSES: ReadonlySet<Status> = new Set<Status>([
  "created",
  "pending",
  "running",
]);

function timestampMs(ts: string | null | undefined): number {
  if (!ts) return 0;
  const t = new Date(ts).getTime();
  return Number.isFinite(t) ? t : 0;
}

/** Active sessions sort key: most recently started (or created) first. */
function activeSortKey(record: SessionSummaryRecord): number {
  return Math.max(
    timestampMs(record.session.start_time),
    timestampMs(record.session.creation_time),
    timestampMs(record.timestamp),
  );
}

/** Terminal sessions sort key: most recent end_time, falling back to record timestamp. */
function terminalSortKey(record: SessionSummaryRecord): number {
  return Math.max(timestampMs(record.session.end_time), timestampMs(record.timestamp));
}

export function isActiveSession(record: SessionSummaryRecord): boolean {
  return ACTIVE_STATUSES.has(record.session.status);
}

/**
 * Sort sessions so active ones (Created/Pending/Running) come first, then
 * terminal ones (Complete/Failed) by end_time descending. The input array is
 * not mutated.
 */
export function sortSessionsActiveFirst(
  records: readonly SessionSummaryRecord[],
): SessionSummaryRecord[] {
  const active: SessionSummaryRecord[] = [];
  const terminal: SessionSummaryRecord[] = [];
  for (const r of records) {
    if (isActiveSession(r)) {
      active.push(r);
    } else {
      terminal.push(r);
    }
  }
  active.sort((a, b) => activeSortKey(b) - activeSortKey(a));
  terminal.sort((a, b) => terminalSortKey(b) - terminalSortKey(a));
  return [...active, ...terminal];
}
