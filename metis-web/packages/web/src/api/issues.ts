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

/** Actor identifier — either a username or a task ID. */
export type ActorId =
  | { Username: string }
  | { Task: string };

/** Typed reference to who performed an operation. */
export type ActorRef =
  | { Authenticated: { actor_id: ActorId } }
  | { System: { worker_name: string; on_behalf_of?: ActorId } }
  | { Automation: { automation_name: string; triggered_by?: ActorRef } };

/** Versioned record wrapping an issue. */
export interface IssueVersionRecord {
  issue_id: string;
  version: number;
  timestamp: string;
  issue: IssueData;
  actor?: ActorRef;
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

export function fetchIssues(): Promise<IssueListResponse> {
  return apiFetch<IssueListResponse>("/api/v1/issues");
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

export interface IssueVersionsResponse {
  versions: IssueVersionRecord[];
}

export function fetchIssueVersions(issueId: string): Promise<IssueVersionsResponse> {
  return apiFetch<IssueVersionsResponse>(
    `/api/v1/issues/${encodeURIComponent(issueId)}/versions`,
  );
}
