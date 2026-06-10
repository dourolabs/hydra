import { useCallback, useMemo, useState } from "react";
import type { DocumentSummaryRecord } from "@hydra/api";
import { useDocumentPathChildren } from "../../features/documents/useDocumentPathChildren";
import { useDocumentSummariesUnderPath } from "../../features/documents/useDocumentSummariesUnderPath";
import styles from "../Sidebar.module.css";
import { TreeNode } from "../SidebarDocumentTree";
import { TreeChevron } from "./TreeChevron";
import { indentStyle, type NodeProps } from "./types";

export function FolderRow({ entry, depth }: NodeProps) {
  const [expanded, setExpanded] = useState(false);
  const toggle = useCallback(() => setExpanded((p) => !p), []);
  const { data } = useDocumentPathChildren(entry.full_path, expanded);
  const children = data?.children ?? [];

  const { data: docsData, isLoading: docsLoading } = useDocumentSummariesUnderPath(
    entry.full_path,
    expanded,
  );
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
