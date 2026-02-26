import type { JobSummary } from "@metis/ui";
import type { JobSummaryRecord } from "@metis/api";

export function toJobSummary(record: JobSummaryRecord): JobSummary {
  const status = record.task.status === "unknown" ? "created" : record.task.status;
  return {
    jobId: record.job_id,
    status,
    startTime: record.task.start_time,
    endTime: record.task.end_time,
  };
}
