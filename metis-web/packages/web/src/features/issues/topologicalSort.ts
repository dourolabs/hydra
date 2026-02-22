import type { IssueSummaryRecord } from "@metis/api";

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
