import type { IssueSummaryRecord } from "@hydra/api";
import type { WorkItem } from "../dashboard/workItemTypes";

/**
 * Topologically sort sibling issues by blocked-on dependencies using Kahn's algorithm.
 *
 * If issue X has a { type: "blocked-on", issue_id: Y } dependency and Y is in the
 * sibling set, then Y will appear before X in the output. Issues at the same
 * topological tier preserve their original input order (stable sort).
 *
 * Cycles are handled gracefully: remaining nodes are appended in input order.
 */
export function topologicalSort(
  issues: IssueSummaryRecord[],
): IssueSummaryRecord[] {
  if (issues.length <= 1) return issues;

  const siblingIds = new Set(issues.map((i) => i.issue_id));
  const indexMap = new Map<string, number>();
  for (let i = 0; i < issues.length; i++) {
    indexMap.set(issues[i].issue_id, i);
  }

  // Build adjacency list and in-degree count.
  // Edge: blocker -> blocked (Y -> X when X is blocked-on Y)
  const adj = new Map<string, string[]>();
  const inDegree = new Map<string, number>();

  for (const id of siblingIds) {
    adj.set(id, []);
    inDegree.set(id, 0);
  }

  for (const issue of issues) {
    for (const dep of issue.issue.dependencies) {
      if (dep.type === "blocked-on" && siblingIds.has(dep.issue_id)) {
        adj.get(dep.issue_id)!.push(issue.issue_id);
        inDegree.set(issue.issue_id, inDegree.get(issue.issue_id)! + 1);
      }
    }
  }

  // Kahn's algorithm with stable tie-breaking by input order.
  const queue: string[] = [];
  for (const issue of issues) {
    if (inDegree.get(issue.issue_id) === 0) {
      queue.push(issue.issue_id);
    }
  }

  const result: IssueSummaryRecord[] = [];
  while (queue.length > 0) {
    const id = queue.shift()!;
    result.push(issues[indexMap.get(id)!]);

    // Sort neighbors by their original input order before processing
    // to maintain stable ordering within the same tier.
    const neighbors = adj.get(id)!;
    for (const neighbor of neighbors) {
      const newDeg = inDegree.get(neighbor)! - 1;
      inDegree.set(neighbor, newDeg);
      if (newDeg === 0) {
        // Insert into queue maintaining input order
        const neighborIdx = indexMap.get(neighbor)!;
        let insertPos = queue.length;
        for (let i = 0; i < queue.length; i++) {
          if (indexMap.get(queue[i])! > neighborIdx) {
            insertPos = i;
            break;
          }
        }
        queue.splice(insertPos, 0, neighbor);
      }
    }
  }

  // Handle cycles: append remaining nodes in their original input order.
  if (result.length < issues.length) {
    const added = new Set(result.map((r) => r.issue_id));
    for (const issue of issues) {
      if (!added.has(issue.issue_id)) {
        result.push(issue);
      }
    }
  }

  return result;
}

/**
 * Topologically sort WorkItems for the active dashboard section.
 *
 * Builds a dependency graph from both child-of and blocked-on edges across
 * all issue WorkItems, then applies Kahn's algorithm so that items expected
 * to complete sooner appear first (leaf/unblocked issues at top, root/parent
 * issues at bottom).
 *
 * Edge construction:
 * - x:child-of:y  → edge x → y (child completes before parent)
 * - x:blocked-on:y → edge y → x (blocker completes before blocked)
 *
 * Only edges where both endpoints are in the active item set are considered.
 * Within each topological tier, items are sorted by lastUpdated descending.
 * Cycles are handled gracefully by appending remaining nodes in lastUpdated
 * order. Non-issue items are appended at the end sorted by lastUpdated.
 */
export function topologicalSortWorkItems(items: WorkItem[]): WorkItem[] {
  if (items.length <= 1) return items;

  const issueItems: WorkItem[] = [];
  const nonIssueItems: WorkItem[] = [];
  for (const item of items) {
    if (item.kind === "issue") {
      issueItems.push(item);
    } else {
      nonIssueItems.push(item);
    }
  }

  if (issueItems.length === 0) {
    return [...nonIssueItems].sort(compareByLastUpdated);
  }

  const activeIds = new Set(issueItems.map((i) => i.id));
  const itemMap = new Map<string, WorkItem>();
  const adj = new Map<string, string[]>();
  const inDegree = new Map<string, number>();

  for (const item of issueItems) {
    itemMap.set(item.id, item);
    adj.set(item.id, []);
    inDegree.set(item.id, 0);
  }

  for (const item of issueItems) {
    if (item.kind !== "issue") continue;
    for (const dep of item.data.issue.dependencies) {
      if (dep.type === "child-of" && activeIds.has(dep.issue_id)) {
        // child completes before parent: edge child → parent
        adj.get(item.id)!.push(dep.issue_id);
        inDegree.set(dep.issue_id, inDegree.get(dep.issue_id)! + 1);
      } else if (dep.type === "blocked-on" && activeIds.has(dep.issue_id)) {
        // blocker completes before blocked: edge blocker → blocked
        adj.get(dep.issue_id)!.push(item.id);
        inDegree.set(item.id, inDegree.get(item.id)! + 1);
      }
    }
  }

  // Kahn's algorithm with tier-aware ordering.
  const compareFn = (aId: string, bId: string): number =>
    compareByLastUpdated(itemMap.get(aId)!, itemMap.get(bId)!);

  let currentTier: string[] = [];
  for (const item of issueItems) {
    if (inDegree.get(item.id) === 0) {
      currentTier.push(item.id);
    }
  }
  currentTier.sort(compareFn);

  const result: WorkItem[] = [];
  while (currentTier.length > 0) {
    const nextTier: string[] = [];
    for (const id of currentTier) {
      result.push(itemMap.get(id)!);
      for (const neighbor of adj.get(id)!) {
        const newDeg = inDegree.get(neighbor)! - 1;
        inDegree.set(neighbor, newDeg);
        if (newDeg === 0) {
          nextTier.push(neighbor);
        }
      }
    }
    nextTier.sort(compareFn);
    currentTier = nextTier;
  }

  // Handle cycles: append remaining nodes sorted by lastUpdated descending.
  if (result.length < issueItems.length) {
    const added = new Set(result.map((r) => r.id));
    const remaining = issueItems
      .filter((i) => !added.has(i.id))
      .sort(compareByLastUpdated);
    result.push(...remaining);
  }

  // Append non-issue items at the end.
  if (nonIssueItems.length > 0) {
    result.push(...nonIssueItems.sort(compareByLastUpdated));
  }

  return result;
}

function compareByLastUpdated(a: WorkItem, b: WorkItem): number {
  return (
    new Date(b.lastUpdated).getTime() - new Date(a.lastUpdated).getTime()
  );
}
