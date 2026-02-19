import { ApiError } from "@metis/api";

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
