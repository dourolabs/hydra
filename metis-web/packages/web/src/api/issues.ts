import { apiFetch } from "./client";

export interface IssueDependency {
  type: "child-of" | "blocked-on";
  issue_id: string;
}

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
  issues: { issue: Issue }[];
}

export function fetchIssues(): Promise<IssueListResponse> {
  return apiFetch<IssueListResponse>("/api/v1/issues");
}
