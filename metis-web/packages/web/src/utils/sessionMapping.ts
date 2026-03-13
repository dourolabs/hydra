import type { SessionSummary } from "@metis/ui";
import type { JobSummaryRecord } from "@metis/api";

export function toSessionSummary(record: JobSummaryRecord): SessionSummary {
  const status = record.task.status === "unknown" ? "created" : record.task.status;
  return {
    sessionId: record.job_id,
    status,
    startTime: record.task.start_time,
    endTime: record.task.end_time,
  };
}
