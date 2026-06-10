import { useDocumentPathChildren } from "../features/documents/useDocumentPathChildren";
import styles from "./Sidebar.module.css";
import { DocumentLeafRow } from "./SidebarDocumentTree/DocumentLeafRow";
import { FolderRow } from "./SidebarDocumentTree/FolderRow";
import { HybridRow } from "./SidebarDocumentTree/HybridRow";
import type { NodeProps } from "./SidebarDocumentTree/types";

const TOP_LEVEL_LIMIT = 10;

export function TreeNode({ entry, depth, pathToDoc, pathToDocLoading }: NodeProps) {
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
