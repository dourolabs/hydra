import { useCallback, useMemo, useState } from "react";
import { NavLink } from "react-router-dom";
import type { DocumentSummaryRecord } from "@hydra/api";
import { useDocumentPathChildren } from "../../features/documents/useDocumentPathChildren";
import { useDocumentSummariesAtPath } from "../../features/documents/useDocumentSummariesAtPath";
import { useDocumentSummariesUnderPath } from "../../features/documents/useDocumentSummariesUnderPath";
import styles from "../Sidebar.module.css";
import { TreeNode } from "../SidebarDocumentTree";
import { TreeChevron } from "./TreeChevron";
import { indentStyle, type NodeProps } from "./types";

function hybridLinkClass({ isActive }: { isActive: boolean }) {
  return `${styles.treeHybridLink}${isActive ? ` ${styles.navItemActive}` : ""}`;
}

export function HybridRow({ entry, depth, pathToDoc, pathToDocLoading }: NodeProps) {
  const [expanded, setExpanded] = useState(false);
  const toggle = useCallback(() => setExpanded((p) => !p), []);

  // Resolve this row's own document for its NavLink. Prefer the parent's
  // batched map when available; otherwise fall back to a per-row lookup
  // (e.g., for a hybrid row at the top level).
  const fallback = useDocumentSummariesAtPath(entry.full_path, pathToDoc === undefined);
  let doc: DocumentSummaryRecord | undefined;
  let docLoading: boolean;
  if (pathToDoc !== undefined) {
    doc = pathToDoc.get(entry.full_path);
    docLoading = pathToDocLoading ?? false;
  } else {
    doc = fallback.data?.documents.find((d) => !d.document.deleted);
    docLoading = fallback.isLoading;
  }

  // Children: same pattern as FolderRow — fetch path children, plus a single
  // batched listDocuments under this path so child leaves resolve from a map.
  const { data: childrenData } = useDocumentPathChildren(entry.full_path, expanded);
  const children = childrenData?.children ?? [];
  const { data: childDocsData, isLoading: childDocsLoading } = useDocumentSummariesUnderPath(
    entry.full_path,
    expanded,
  );
  const childPathToDoc = useMemo(() => {
    const map = new Map<string, DocumentSummaryRecord>();
    for (const record of childDocsData?.documents ?? []) {
      if (record.document.deleted) continue;
      const path = record.document.path;
      if (path == null) continue;
      if (!map.has(path)) map.set(path, record);
    }
    return map;
  }, [childDocsData]);

  return (
    <>
      <div className={styles.treeHybrid} style={indentStyle(depth)}>
        <button
          type="button"
          className={styles.treeHybridChevron}
          onClick={toggle}
          aria-expanded={expanded}
          aria-label={expanded ? "Collapse" : "Expand"}
          data-testid={`sidebar-doc-tree-hybrid-${entry.full_path}`}
        >
          <TreeChevron expanded={expanded} />
        </button>
        {docLoading || !doc ? (
          <div
            className={styles.treeHybridPlaceholder}
            data-testid={`sidebar-doc-tree-leaf-loading-${entry.full_path}`}
            title={entry.name}
          >
            {entry.name}
          </div>
        ) : (
          <NavLink
            to={`/documents/${doc.document_id}`}
            className={hybridLinkClass}
            data-testid={`sidebar-doc-tree-leaf-${doc.document_id}`}
            title={entry.name}
          >
            {entry.name}
          </NavLink>
        )}
      </div>
      {expanded &&
        children.map((child) => (
          <TreeNode
            key={child.full_path}
            entry={child}
            depth={depth + 1}
            pathToDoc={childPathToDoc}
            pathToDocLoading={childDocsLoading}
          />
        ))}
    </>
  );
}
