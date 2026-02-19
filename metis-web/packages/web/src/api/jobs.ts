import { apiFetch, ApiError } from "./client";

/** Nested task data inside a JobVersionRecord. */
export interface TaskData {
  status: string;
  spawned_from?: string;
  creator: string;
  creation_time?: string;
  start_time?: string;
  end_time?: string;
}

/** Server response shape: versioned record wrapping a Task. */
export interface JobVersionRecord {
  job_id: string;
  version: number;
  timestamp: string;
  task: TaskData;
}

/** Flattened job type used throughout the UI. */
export interface Job {
  job_id: string;
  status: string;
  spawned_from: string;
  creation_time: string | null;
  start_time: string | null;
  end_time: string | null;
}

export interface ListJobsResponse {
  jobs: JobVersionRecord[];
}

/** Convert a JobVersionRecord to the flat Job type used in the UI. */
export function toJob(record: JobVersionRecord): Job {
  return {
    job_id: record.job_id,
    status: record.task.status,
    spawned_from: record.task.spawned_from ?? "",
    creation_time: record.task.creation_time ?? null,
    start_time: record.task.start_time ?? null,
    end_time: record.task.end_time ?? null,
  };
}

export function fetchJobsByIssue(issueId: string): Promise<ListJobsResponse> {
  return apiFetch<ListJobsResponse>(
    `/api/v1/jobs?spawned_from=${encodeURIComponent(issueId)}`,
  );
}

/** Fetch a single job by ID. */
export function fetchJob(jobId: string): Promise<JobVersionRecord> {
  return apiFetch<JobVersionRecord>(
    `/api/v1/jobs/${encodeURIComponent(jobId)}`,
  );
}

/** Fetch full log output for a completed job (plain text). */
export async function fetchJobLogs(jobId: string): Promise<string> {
  const resp = await fetch(
    `/api/v1/jobs/${encodeURIComponent(jobId)}/logs`,
  );
  if (!resp.ok) {
    const body = await resp.text().catch(() => resp.statusText);
    throw new ApiError(resp.status, body || resp.statusText);
  }
  return resp.text();
}

/**
 * Stream job logs via SSE for a running job.
 * Returns an EventSource instance. The caller is responsible for closing it.
 * Each "message" event's `data` field contains a chunk of log text.
 */
export function streamJobLogs(jobId: string): EventSource {
  return new EventSource(
    `/api/v1/jobs/${encodeURIComponent(jobId)}/logs?watch=true`,
  );
}
