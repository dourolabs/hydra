import { useCallback, useMemo, useState } from "react";
import { NavLink } from "react-router-dom";
import type { DocumentSummaryRecord, PathChildEntry } from "@hydra/api";
import { useDocumentPathChildren } from "../features/documents/useDocumentPathChildren";
import { useDocumentSummariesAtPath } from "../features/documents/useDocumentSummariesAtPath";
import { useDocumentSummariesUnderPath } from "../features/documents/useDocumentSummariesUnderPath";
import styles from "./Sidebar.module.css";

const TOP_LEVEL_LIMIT = 10;
const INDENT_STEP_PX = 12;

function TreeChevron({ expanded }: { expanded: boolean }) {
  return (
    <svg
      className={`${styles.treeChevron}${expanded ? ` ${styles.treeChevronOpen}` : ""}`}
      viewBox="0 0 20 20"
      fill="currentColor"
      aria-hidden="true"
    >
      <path
        fillRule="evenodd"
        d="M7.21 14.77a.75.75 0 01.02-1.06L11.168 10 7.23 6.29a.75.75 0 111.04-1.08l4.5 4.25a.75.75 0 010 1.08l-4.5 4.25a.75.75 0 01-1.06-.02z"
        clipRule="evenodd"
      />
    </svg>
  );
}

function indentStyle(depth: number) {
  return { paddingLeft: `${depth * INDENT_STEP_PX + 8}px` } as const;
}

function leafLinkClass({ isActive }: { isActive: boolean }) {
  return `${styles.treeLeaf}${isActive ? ` ${styles.navItemActive}` : ""}`;
}

function hybridLinkClass({ isActive }: { isActive: boolean }) {
  return `${styles.treeHybridLink}${isActive ? ` ${styles.navItemActive}` : ""}`;
}

interface NodeProps {
  entry: PathChildEntry;
  depth: number;
  pathToDoc?: Map<string, DocumentSummaryRecord>;
  pathToDocLoading?: boolean;
}

function DocumentLeafRow({
  entry,
  depth,
  pathToDoc,
  pathToDocLoading,
}: NodeProps) {
  const fallback = useDocumentSummariesAtPath(
    entry.full_path,
    pathToDoc === undefined,
  );

  let doc: DocumentSummaryRecord | undefined;
  let isLoading: boolean;
  if (pathToDoc !== undefined) {
    doc = pathToDoc.get(entry.full_path);
    isLoading = pathToDocLoading ?? false;
  } else {
    doc = fallback.data?.documents.find((d) => !d.document.deleted);
    isLoading = fallback.isLoading;
  }

  if (isLoading || !doc) {
    return (
      <div
        className={styles.treeLeafPlaceholder}
        style={indentStyle(depth)}
        data-testid={`sidebar-doc-tree-leaf-loading-${entry.full_path}`}
        title={entry.name}
      >
        {entry.name}
      </div>
    );
  }

  return (
    <NavLink
      to={`/documents/${doc.document_id}`}
      className={leafLinkClass}
      style={indentStyle(depth)}
      data-testid={`sidebar-doc-tree-leaf-${doc.document_id}`}
      title={entry.name}
    >
      {entry.name}
    </NavLink>
  );
}

function FolderRow({ entry, depth }: NodeProps) {
  const [expanded, setExpanded] = useState(false);
  const toggle = useCallback(() => setExpanded((p) => !p), []);
  const { data } = useDocumentPathChildren(entry.full_path, expanded);
  const children = data?.children ?? [];

  const { data: docsData, isLoading: docsLoading } =
    useDocumentSummariesUnderPath(entry.full_path, expanded);
  const pathToDoc = useMemo(() => {
    const map = new Map<string, DocumentSummaryRecord>();
    for (const record of docsData?.documents ?? []) {
      if (record.document.deleted) continue;
      const path = record.document.path;
      if (path == null) continue;
      if (!map.has(path)) map.set(path, record);
    }
    return map;
  }, [docsData]);

  return (
    <>
      <button
        type="button"
        className={styles.treeFolder}
        style={indentStyle(depth)}
        onClick={toggle}
        aria-expanded={expanded}
        data-testid={`sidebar-doc-tree-folder-${entry.full_path}`}
        title={entry.name}
      >
        <TreeChevron expanded={expanded} />
        <span className={styles.treeFolderName}>{entry.name}</span>
      </button>
      {expanded &&
        children.map((child) => (
          <TreeNode
            key={child.full_path}
            entry={child}
            depth={depth + 1}
            pathToDoc={pathToDoc}
            pathToDocLoading={docsLoading}
          />
        ))}
    </>
  );
}

function HybridRow({ entry, depth, pathToDoc, pathToDocLoading }: NodeProps) {
  const [expanded, setExpanded] = useState(false);
  const toggle = useCallback(() => setExpanded((p) => !p), []);

  // Resolve this row's own document for its NavLink. Prefer the parent's
  // batched map when available; otherwise fall back to a per-row lookup
  // (e.g., for a hybrid row at the top level).
  const fallback = useDocumentSummariesAtPath(
    entry.full_path,
    pathToDoc === undefined,
  );
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
  const { data: childrenData } = useDocumentPathChildren(
    entry.full_path,
    expanded,
  );
  const children = childrenData?.children ?? [];
  const { data: childDocsData, isLoading: childDocsLoading } =
    useDocumentSummariesUnderPath(entry.full_path, expanded);
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

function TreeNode({ entry, depth, pathToDoc, pathToDocLoading }: NodeProps) {
  // Pure document (no descendants beyond itself): leaf row.
  if (entry.is_document && Number(entry.child_count) <= 1) {
    return (
      <DocumentLeafRow
        entry={entry}
        depth={depth}
        pathToDoc={pathToDoc}
        pathToDocLoading={pathToDocLoading}
      />
    );
  }
  // Hybrid (document with descendants): chevron + link row.
  if (entry.is_document && Number(entry.child_count) > 1) {
    return (
      <HybridRow
        entry={entry}
        depth={depth}
        pathToDoc={pathToDoc}
        pathToDocLoading={pathToDocLoading}
      />
    );
  }
  return <FolderRow entry={entry} depth={depth} />;
}

export function SidebarDocumentTree() {
  const { data, isLoading } = useDocumentPathChildren(null);
  const entries = (data?.children ?? []).slice(0, TOP_LEVEL_LIMIT);

  if (isLoading) {
    return null;
  }
  if (entries.length === 0) {
    return null;
  }

  return (
    <div className={styles.docTree} data-testid="sidebar-doc-tree">
      {entries.map((entry) => (
        <TreeNode key={entry.full_path} entry={entry} depth={0} />
      ))}
    </div>
  );
}
