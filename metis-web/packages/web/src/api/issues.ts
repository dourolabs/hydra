import { apiFetch } from "./client";

export interface IssueDependency {
  type: "child-of" | "blocked-on";
  issue_id: string;
}

/** The inner issue object as returned by the metis-server API. */
export interface IssueData {
  type: string;
  description: string;
  status: string;
  assignee: string | null;
  creator: string;
  progress: string;
  dependencies: IssueDependency[];
  patches: string[];
  /** May be omitted in JSON when the list is empty. */
  todo_list?: { description: string; is_done: boolean }[];
}

/** Versioned record wrapping an issue. */
export interface IssueVersionRecord {
  issue_id: string;
  version: number;
  timestamp: string;
  issue: IssueData;
}

/** Flattened issue type used throughout the UI. */
export interface Issue {
  issue_id: string;
  description: string;
  status: string;
  assignee: string | null;
  creator: string;
  type: string;
  progress: string;
  dependencies: IssueDependency[];
  timestamp: string;
}

export interface IssueListResponse {
  issues: IssueVersionRecord[];
}

/** Convert a versioned record to the flat Issue type used in the UI. */
export function toIssue(record: IssueVersionRecord): Issue {
  return {
    issue_id: record.issue_id,
    description: record.issue.description,
    status: record.issue.status,
    assignee: record.issue.assignee,
    creator: record.issue.creator,
    type: record.issue.type,
    progress: record.issue.progress,
    dependencies: record.issue.dependencies,
    timestamp: record.timestamp,
  };
}

export interface SearchIssuesQuery {
  q?: string;
}

export function fetchIssues(query?: SearchIssuesQuery): Promise<IssueListResponse> {
  const params = new URLSearchParams();
  if (query?.q) {
    params.set("q", query.q);
  }
  const qs = params.toString();
  return apiFetch<IssueListResponse>(`/api/v1/issues${qs ? `?${qs}` : ""}`);
}

export function fetchIssue(issueId: string): Promise<IssueVersionRecord> {
  return apiFetch<IssueVersionRecord>(`/api/v1/issues/${encodeURIComponent(issueId)}`);
}

export interface CreateIssueParams {
  description: string;
  creator: string;
  assignee?: string;
  type?: string;
  repoName?: string;
}

export interface CreateIssueResponse {
  issue_id: string;
  version: number;
}

export function createIssue(params: CreateIssueParams): Promise<CreateIssueResponse> {
  const issue: Record<string, unknown> = {
    type: params.type ?? "task",
    description: params.description,
    creator: params.creator,
  };

  if (params.assignee) {
    issue.assignee = params.assignee;
  }

  if (params.repoName) {
    issue.job_settings = { repo_name: params.repoName };
  }

  return apiFetch<CreateIssueResponse>("/api/v1/issues", {
    method: "POST",
    body: JSON.stringify({ issue }),
  });
}
