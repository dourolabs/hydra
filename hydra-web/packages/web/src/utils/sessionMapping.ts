import type { SessionSummary } from "@hydra/ui";
import type { SessionSummaryRecord } from "@hydra/api";

export function toSessionSummary(record: SessionSummaryRecord): SessionSummary {
  const status = record.session.status === "unknown" ? "created" : record.session.status;
  return {
    sessionId: record.session_id,
    status,
    startTime: record.session.start_time,
    endTime: record.session.end_time,
  };
}
