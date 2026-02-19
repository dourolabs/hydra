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
  todo_list: { description: string; is_done: boolean }[];
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
  };
}

export function fetchIssues(): Promise<IssueListResponse> {
  return apiFetch<IssueListResponse>("/api/v1/issues");
}
