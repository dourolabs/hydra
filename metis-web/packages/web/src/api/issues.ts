import type { UpsertIssueResponse } from "@metis/api";
import { apiClient } from "./client";

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
