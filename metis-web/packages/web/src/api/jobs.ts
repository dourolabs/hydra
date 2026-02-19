import { apiFetch } from "./client";

export interface Job {
  job_id: string;
  status: string;
  spawned_from: string;
  creation_time: string;
  start_time: string | null;
  end_time: string | null;
}

export interface ListJobsResponse {
  jobs: Job[];
}

export function fetchJobsByIssue(issueId: string): Promise<ListJobsResponse> {
  return apiFetch<ListJobsResponse>(
    `/api/v1/jobs?spawned_from=${encodeURIComponent(issueId)}`,
  );
}
