import { createContext, useCallback, useContext, useMemo, useState } from "react";
import { NavLink } from "react-router-dom";
import type { PathChildEntry } from "@hydra/api";
import {
  useBatchedDocumentPaths,
  type BatchedDocumentPaths,
} from "../features/documents/useBatchedDocumentPaths";
import styles from "./Sidebar.module.css";

const TOP_LEVEL_LIMIT = 10;
const INDENT_STEP_PX = 12;

interface TreeContext {
  expanded: Set<string>;
  toggle: (prefix: string) => void;
  batched: BatchedDocumentPaths;
}

const SidebarTreeContext = createContext<TreeContext | null>(null);

function useTreeContext(): TreeContext {
  const ctx = useContext(SidebarTreeContext);
  if (!ctx) {
    throw new Error("SidebarTreeContext missing");
  }
  return ctx;
}

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
}

function DocumentLeafRow({ entry, depth }: NodeProps) {
  const doc = entry.document ?? undefined;

  if (!doc) {
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
  const { expanded, toggle, batched } = useTreeContext();
  const isOpen = expanded.has(entry.full_path);
  const onToggle = useCallback(
    () => toggle(entry.full_path),
    [entry.full_path, toggle],
  );
  const children = isOpen ? batched.getChildren(entry.full_path) : [];

  return (
    <>
      <button
        type="button"
        className={styles.treeFolder}
        style={indentStyle(depth)}
        onClick={onToggle}
        aria-expanded={isOpen}
        data-testid={`sidebar-doc-tree-folder-${entry.full_path}`}
        title={entry.name}
      >
        <TreeChevron expanded={isOpen} />
        <span className={styles.treeFolderName}>{entry.name}</span>
      </button>
      {isOpen &&
        children.map((child) => (
          <TreeNode key={child.full_path} entry={child} depth={depth + 1} />
        ))}
    </>
  );
}

function HybridRow({ entry, depth }: NodeProps) {
  const { expanded, toggle, batched } = useTreeContext();
  const isOpen = expanded.has(entry.full_path);
  const onToggle = useCallback(
    () => toggle(entry.full_path),
    [entry.full_path, toggle],
  );
  const doc = entry.document ?? undefined;
  const children = isOpen ? batched.getChildren(entry.full_path) : [];

  return (
    <>
      <div className={styles.treeHybrid} style={indentStyle(depth)}>
        <button
          type="button"
          className={styles.treeHybridChevron}
          onClick={onToggle}
          aria-expanded={isOpen}
          aria-label={isOpen ? "Collapse" : "Expand"}
          data-testid={`sidebar-doc-tree-hybrid-${entry.full_path}`}
        >
          <TreeChevron expanded={isOpen} />
        </button>
        {!doc ? (
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
      {isOpen &&
        children.map((child) => (
          <TreeNode key={child.full_path} entry={child} depth={depth + 1} />
        ))}
    </>
  );
}

function TreeNode({ entry, depth }: NodeProps) {
  if (entry.is_document && Number(entry.child_count) <= 1) {
    return <DocumentLeafRow entry={entry} depth={depth} />;
  }
  if (entry.is_document && Number(entry.child_count) > 1) {
    return <HybridRow entry={entry} depth={depth} />;
  }
  return <FolderRow entry={entry} depth={depth} />;
}

export function SidebarDocumentTree() {
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());

  const toggle = useCallback((prefix: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(prefix)) {
        next.delete(prefix);
      } else {
        next.add(prefix);
      }
      return next;
    });
  }, []);

  const prefixes = useMemo<Array<string | null>>(
    () => [null, ...expanded],
    [expanded],
  );

  const batched = useBatchedDocumentPaths(prefixes);
  const topLevel = useMemo(
    () => batched.getChildren(null).slice(0, TOP_LEVEL_LIMIT),
    [batched],
  );

  const contextValue = useMemo<TreeContext>(
    () => ({ expanded, toggle, batched }),
    [expanded, toggle, batched],
  );

  if (batched.isLoading && !batched.data) {
    return null;
  }
  if (topLevel.length === 0) {
    return null;
  }

  return (
    <SidebarTreeContext.Provider value={contextValue}>
      <div className={styles.docTree} data-testid="sidebar-doc-tree">
        {topLevel.map((entry) => (
          <TreeNode key={entry.full_path} entry={entry} depth={0} />
        ))}
      </div>
    </SidebarTreeContext.Provider>
  );
}
