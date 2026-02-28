import type {
  IssueSummaryRecord,
  JobSummaryRecord,
  PatchSummaryRecord,
} from "@metis/api";
import type { IssueTreeNode } from "../issues/useIssues";

export type PipelineStage =
  | "open"
  | "agent-working"
  | "awaiting-review"
  | "done"
  | "failed";

export interface ClassifiedIssue {
  issue: IssueSummaryRecord;
  stage: PipelineStage;
  jobs: JobSummaryRecord[];
}

export interface PipelineStageCounts {
  open: ClassifiedIssue[];
  "agent-working": ClassifiedIssue[];
  "awaiting-review": ClassifiedIssue[];
  done: ClassifiedIssue[];
  failed: ClassifiedIssue[];
}

export const STAGE_ORDER: PipelineStage[] = [
  "open",
  "agent-working",
  "awaiting-review",
  "done",
  "failed",
];

export const STAGE_LABELS: Record<PipelineStage, string> = {
  open: "Open",
  "agent-working": "Agent working",
  "awaiting-review": "Awaiting review",
  done: "Done",
  failed: "Failed",
};

const FAILED_STATUSES = new Set(["failed", "rejected", "dropped"]);

function hasRunningOrPendingJob(jobs: JobSummaryRecord[]): boolean {
  return jobs.some(
    (j) => j.task.status === "running" || j.task.status === "pending",
  );
}

function hasOpenOrChangesRequestedPatch(
  issue: IssueSummaryRecord,
  patchMap: Map<string, PatchSummaryRecord>,
): boolean {
  for (const patchId of issue.issue.patches) {
    const patch = patchMap.get(patchId);
    if (
      patch &&
      (patch.patch.status === "Open" ||
        patch.patch.status === "ChangesRequested")
    ) {
      return true;
    }
  }
  return false;
}

export function classifyStage(
  issue: IssueSummaryRecord,
  jobs: JobSummaryRecord[],
  patchMap: Map<string, PatchSummaryRecord>,
): PipelineStage {
  const status = issue.issue.status;

  if (status === "closed") return "done";
  if (FAILED_STATUSES.has(status)) return "failed";

  if (status === "in-progress") {
    if (hasRunningOrPendingJob(jobs)) return "agent-working";
    if (hasOpenOrChangesRequestedPatch(issue, patchMap)) return "awaiting-review";
  }

  return "open";
}

/**
 * Walk a tree, collecting all non-root descendants and classifying them.
 */
function collectDescendants(
  node: IssueTreeNode,
  jobsByIssue: Map<string, JobSummaryRecord[]>,
  patchMap: Map<string, PatchSummaryRecord>,
  result: ClassifiedIssue[],
): void {
  for (const child of node.children) {
    if (child.hardBlocked) continue;
    const jobs = jobsByIssue.get(child.id) ?? [];
    result.push({
      issue: child.issue,
      stage: classifyStage(child.issue, jobs, patchMap),
      jobs,
    });
    collectDescendants(child, jobsByIssue, patchMap, result);
  }
}

export function classifySubtasks(
  root: IssueTreeNode,
  jobsByIssue: Map<string, JobSummaryRecord[]>,
  patchMap: Map<string, PatchSummaryRecord>,
): PipelineStageCounts {
  const all: ClassifiedIssue[] = [];
  collectDescendants(root, jobsByIssue, patchMap, all);

  const counts: PipelineStageCounts = {
    open: [],
    "agent-working": [],
    "awaiting-review": [],
    done: [],
    failed: [],
  };

  for (const item of all) {
    counts[item.stage].push(item);
  }

  return counts;
}

/**
 * Count all running/pending jobs across a tree (root + all descendants).
 */
export function countActiveJobs(
  root: IssueTreeNode,
  jobsByIssue: Map<string, JobSummaryRecord[]>,
): number {
  let count = 0;

  function walk(node: IssueTreeNode) {
    const jobs = jobsByIssue.get(node.id) ?? [];
    count += jobs.filter(
      (j) => j.task.status === "running" || j.task.status === "pending",
    ).length;
    for (const child of node.children) {
      walk(child);
    }
  }

  walk(root);
  return count;
}

/**
 * Compute overall progress as percentage of done subtasks.
 */
export function computeProgress(counts: PipelineStageCounts): number {
  const total =
    counts.open.length +
    counts["agent-working"].length +
    counts["awaiting-review"].length +
    counts.done.length +
    counts.failed.length;
  if (total === 0) return 0;
  return Math.round((counts.done.length / total) * 100);
}
