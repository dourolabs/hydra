import { useMemo } from "react";
import type {
  IssueSummaryRecord,
  PatchSummaryRecord,
  DocumentSummaryRecord,
} from "@metis/api";
import { TERMINAL_STATUSES } from "../../utils/statusMapping";
import { usePatchesByIssue } from "../patches/usePatchesByIssue";
import { useDocuments } from "../documents/useDocuments";

// ---------------------------------------------------------------------------
// WorkItem discriminated union
// ---------------------------------------------------------------------------

export type WorkItem =
  | {
      kind: "issue";
      id: string;
      data: IssueSummaryRecord;
      lastUpdated: string;
      isTerminal: boolean;
      hasInProgressChild: boolean;
    }
  | {
      kind: "patch";
      id: string;
      data: PatchSummaryRecord;
      lastUpdated: string;
      isTerminal: boolean;
    }
  | {
      kind: "document";
      id: string;
      data: DocumentSummaryRecord;
      lastUpdated: string;
      isTerminal: boolean;
    };

// ---------------------------------------------------------------------------
// Terminal patch statuses (Merged and Closed are terminal)
// ---------------------------------------------------------------------------

const TERMINAL_PATCH_STATUSES: ReadonlySet<string> = new Set([
  "Merged",
  "Closed",
]);

// ---------------------------------------------------------------------------
// DOC_PATH_RE — detect document paths in issue text
// ---------------------------------------------------------------------------

export const DOC_PATH_RE = /(?:^|\s)(\/\S+\.md)/gm;

/**
 * Extract all unique document paths from a text string.
 */
export function extractDocumentPaths(text: string): string[] {
  const paths = new Set<string>();
  let match: RegExpExecArray | null;
  // Reset lastIndex since we use the global flag
  DOC_PATH_RE.lastIndex = 0;
  while ((match = DOC_PATH_RE.exec(text)) !== null) {
    paths.add(match[1]);
  }
  return Array.from(paths);
}

// ---------------------------------------------------------------------------
// Pure functions for graph traversal and item collection
// ---------------------------------------------------------------------------

/**
 * Find all transitive children of a root issue via "child-of" edges,
 * including the root itself. Returns an array of issue IDs.
 */
export function findTransitiveChildren(
  rootId: string,
  issues: IssueSummaryRecord[],
): string[] {
  // Build parent -> children map
  const childrenMap = new Map<string, string[]>();
  for (const issue of issues) {
    for (const dep of issue.issue.dependencies) {
      if (dep.type === "child-of") {
        const siblings = childrenMap.get(dep.issue_id) ?? [];
        siblings.push(issue.issue_id);
        childrenMap.set(dep.issue_id, siblings);
      }
    }
  }

  const result: string[] = [];
  const visited = new Set<string>();

  function walk(id: string) {
    if (visited.has(id)) return;
    visited.add(id);
    result.push(id);
    for (const childId of childrenMap.get(id) ?? []) {
      walk(childId);
    }
  }

  walk(rootId);
  return result;
}

/**
 * Find all root issue IDs (issues with no "child-of" dependency).
 */
export function findRootIssueIds(issues: IssueSummaryRecord[]): string[] {
  return issues
    .filter(
      (issue) =>
        !issue.issue.dependencies.some((dep) => dep.type === "child-of"),
    )
    .map((issue) => issue.issue_id);
}

/**
 * Collect all patch IDs from a set of issue IDs.
 */
export function collectPatchIds(
  issueIds: string[],
  issueMap: Map<string, IssueSummaryRecord>,
): string[] {
  const patchIds: string[] = [];
  const seen = new Set<string>();
  for (const issueId of issueIds) {
    const issue = issueMap.get(issueId);
    if (!issue) continue;
    for (const patchId of issue.issue.patches) {
      if (!seen.has(patchId)) {
        seen.add(patchId);
        patchIds.push(patchId);
      }
    }
  }
  return patchIds;
}

/**
 * Collect all document paths from issue description and progress text
 * for a set of issue IDs.
 *
 * Note: IssueSummaryRecord truncates description and excludes progress,
 * so document path detection is best-effort from available text.
 */
export function collectDocumentPaths(
  issueIds: string[],
  issueMap: Map<string, IssueSummaryRecord>,
): string[] {
  const allPaths = new Set<string>();
  for (const issueId of issueIds) {
    const issue = issueMap.get(issueId);
    if (!issue) continue;
    const text = issue.issue.description;
    for (const path of extractDocumentPaths(text)) {
      allPaths.add(path);
    }
  }
  return Array.from(allPaths);
}

/**
 * Build WorkItem[] from issues, patches, and documents.
 */
export function buildWorkItems(
  issueIds: string[],
  issueMap: Map<string, IssueSummaryRecord>,
  patches: PatchSummaryRecord[],
  documents: DocumentSummaryRecord[],
  documentPaths: string[],
): WorkItem[] {
  const items: WorkItem[] = [];

  // Build parent -> children map for in-progress child detection
  const childrenMap = new Map<string, string[]>();
  for (const issue of issueMap.values()) {
    for (const dep of issue.issue.dependencies) {
      if (dep.type === "child-of") {
        const siblings = childrenMap.get(dep.issue_id) ?? [];
        siblings.push(issue.issue_id);
        childrenMap.set(dep.issue_id, siblings);
      }
    }
  }

  // Issue work items
  for (const issueId of issueIds) {
    const issue = issueMap.get(issueId);
    if (!issue) continue;
    const childIds = childrenMap.get(issueId) ?? [];
    const hasInProgressChild = childIds.some((childId) => {
      const child = issueMap.get(childId);
      return child?.issue.status === "in-progress";
    });
    items.push({
      kind: "issue",
      id: issue.issue_id,
      data: issue,
      lastUpdated: issue.timestamp,
      isTerminal: TERMINAL_STATUSES.has(issue.issue.status),
      hasInProgressChild,
    });
  }

  // Patch work items
  for (const patch of patches) {
    items.push({
      kind: "patch",
      id: patch.patch_id,
      data: patch,
      lastUpdated: patch.timestamp,
      isTerminal: TERMINAL_PATCH_STATUSES.has(patch.patch.status),
    });
  }

  // Document work items — match by path
  const pathSet = new Set(documentPaths);
  for (const doc of documents) {
    if (doc.document.path && pathSet.has(doc.document.path)) {
      items.push({
        kind: "document",
        id: doc.document_id,
        data: doc,
        lastUpdated: doc.timestamp,
        isTerminal: false, // documents are never terminal
      });
    }
  }

  return items;
}

// ---------------------------------------------------------------------------
// React hook
// ---------------------------------------------------------------------------

export function useTransitiveWorkItems(
  rootIssueId: string | null,
  issues: IssueSummaryRecord[],
) {
  // Build issue map for O(1) lookups
  const issueMap = useMemo(() => {
    const map = new Map<string, IssueSummaryRecord>();
    for (const issue of issues) {
      map.set(issue.issue_id, issue);
    }
    return map;
  }, [issues]);

  // Find transitive issue IDs under the selected root
  const transitiveIssueIds = useMemo(() => {
    if (rootIssueId) {
      return findTransitiveChildren(rootIssueId, issues);
    }
    // "All Items" mode: collect transitive children across all roots
    const rootIds = findRootIssueIds(issues);
    const allIds = new Set<string>();
    for (const rootId of rootIds) {
      for (const id of findTransitiveChildren(rootId, issues)) {
        allIds.add(id);
      }
    }
    return Array.from(allIds);
  }, [rootIssueId, issues]);

  // Collect patch IDs from transitive issues
  const patchIds = useMemo(
    () => collectPatchIds(transitiveIssueIds, issueMap),
    [transitiveIssueIds, issueMap],
  );

  // Collect document paths from transitive issues
  const documentPaths = useMemo(
    () => collectDocumentPaths(transitiveIssueIds, issueMap),
    [transitiveIssueIds, issueMap],
  );

  // Fetch patches
  const {
    data: patches,
    isLoading: patchesLoading,
    error: patchesError,
  } = usePatchesByIssue(patchIds);

  // Fetch all documents
  const {
    data: allDocuments,
    isLoading: documentsLoading,
    error: documentsError,
  } = useDocuments();

  // Build unified work items
  const items = useMemo(
    () =>
      buildWorkItems(
        transitiveIssueIds,
        issueMap,
        patches,
        allDocuments ?? [],
        documentPaths,
      ),
    [transitiveIssueIds, issueMap, patches, allDocuments, documentPaths],
  );

  return {
    items,
    isLoading: patchesLoading || documentsLoading,
    error: patchesError ?? documentsError ?? null,
  };
}
