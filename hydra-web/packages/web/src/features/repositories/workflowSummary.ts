import type { RepositoryRecord } from "@hydra/api";

export function workflowSummary(record: RepositoryRecord): string | null {
  const pw = record.repository.patch_workflow;
  const reviewerCount = pw?.review_requests?.length ?? 0;
  const hasMerge = !!pw?.merge_request?.assignee;
  const parts: string[] = [];
  if (reviewerCount > 0) {
    parts.push(`${reviewerCount} reviewer${reviewerCount === 1 ? "" : "s"}`);
  }
  if (hasMerge) parts.push("merge");
  return parts.length > 0 ? parts.join(", ") : null;
}
