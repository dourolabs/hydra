import type { SessionSummaryRecord } from "@hydra/api";

const ACTIVE_STATUSES = new Set(["created", "pending", "running"]);

function toMillis(value: string | null | undefined): number | null {
  if (!value) return null;
  const t = new Date(value).getTime();
  return Number.isNaN(t) ? null : t;
}

/**
 * Sort sessions with active sessions first (by start_time/creation_time desc),
 * then terminal sessions (by end_time desc, falling back to timestamp).
 */
export function sortSessions(
  sessions: readonly SessionSummaryRecord[],
): SessionSummaryRecord[] {
  const active: SessionSummaryRecord[] = [];
  const terminal: SessionSummaryRecord[] = [];

  for (const record of sessions) {
    if (ACTIVE_STATUSES.has(record.session.status)) {
      active.push(record);
    } else {
      terminal.push(record);
    }
  }

  active.sort((a, b) => {
    const aTime =
      toMillis(a.session.start_time) ??
      toMillis(a.session.creation_time) ??
      toMillis(a.timestamp) ??
      0;
    const bTime =
      toMillis(b.session.start_time) ??
      toMillis(b.session.creation_time) ??
      toMillis(b.timestamp) ??
      0;
    return bTime - aTime;
  });

  terminal.sort((a, b) => {
    const aTime =
      toMillis(a.session.end_time) ?? toMillis(a.timestamp) ?? 0;
    const bTime =
      toMillis(b.session.end_time) ?? toMillis(b.timestamp) ?? 0;
    return bTime - aTime;
  });

  return [...active, ...terminal];
}
