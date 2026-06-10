import { NavLink } from "react-router-dom";
import type { DocumentSummaryRecord } from "@hydra/api";
import { useDocumentSummariesAtPath } from "../../features/documents/useDocumentSummariesAtPath";
import styles from "../Sidebar.module.css";
import { indentStyle, type NodeProps } from "./types";

function leafLinkClass({ isActive }: { isActive: boolean }) {
  return `${styles.treeLeaf}${isActive ? ` ${styles.navItemActive}` : ""}`;
}

export function DocumentLeafRow({ entry, depth, pathToDoc, pathToDocLoading }: NodeProps) {
  const fallback = useDocumentSummariesAtPath(entry.full_path, pathToDoc === undefined);

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
