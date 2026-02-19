import type {
  IssueVersionRecord,
  ListIssuesResponse,
  IssueDependency,
  UpsertIssueResponse,
} from "@metis/api";
import { apiClient } from "./client";

export type { IssueVersionRecord, ListIssuesResponse, IssueDependency };

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

/** Convert a versioned record to the flat Issue type used in the UI. */
export function toIssue(record: IssueVersionRecord): Issue {
  return {
    issue_id: record.issue_id,
    description: record.issue.description,
    status: record.issue.status,
    assignee: record.issue.assignee ?? null,
    creator: record.issue.creator,
    type: record.issue.type,
    progress: record.issue.progress,
    dependencies: record.issue.dependencies,
    timestamp: record.timestamp,
  };
}

export function fetchIssues(): Promise<ListIssuesResponse> {
  return apiClient.listIssues();
}

export function fetchIssue(issueId: string): Promise<IssueVersionRecord> {
  return apiClient.getIssue(issueId);
}

export interface CreateIssueParams {
  description: string;
  creator: string;
  assignee?: string;
  type?: string;
  repoName?: string;
}

export function createIssue(params: CreateIssueParams): Promise<UpsertIssueResponse> {
  return apiClient.createIssue({
    issue: {
      type: (params.type ?? "task") as "task",
      description: params.description,
      creator: params.creator,
      progress: "",
      status: "open",
      dependencies: [],
      patches: [],
      ...(params.assignee && { assignee: params.assignee }),
      ...(params.repoName && { job_settings: { repo_name: params.repoName } }),
    },
    job_id: null,
  });
}
