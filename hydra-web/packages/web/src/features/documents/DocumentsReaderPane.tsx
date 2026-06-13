import { useMemo } from "react";
import { Link } from "react-router-dom";
import { Icons, Spinner } from "@hydra/ui";
import type { DocumentSummaryRecord, PathChildEntry } from "@hydra/api";
import { AgoTime } from "../../components/Runtime/Runtime";
import { isFolderEntry, isLeafDocumentEntry } from "./DocumentTree";
import { useDocumentsByIds } from "./useDocumentsByIds";
import { useUncategorizedDocuments } from "./useUncategorizedDocuments";
import { getDocumentDisplayTitle } from "./utils";
import styles from "./DocumentsReaderPane.module.css";

const ROOT_PATH = "/";

interface BreadcrumbItem {
  name: string;
  path: string;
}

function pathBreadcrumbs(activePath: string): BreadcrumbItem[] {
  if (activePath === ROOT_PATH) return [];
  const segs = activePath.split("/").filter(Boolean);
  const out: BreadcrumbItem[] = [];
  let cur = "";
  for (const s of segs) {
    cur += "/" + s;
    out.push({ name: s, path: cur });
  }
  return out;
}

interface DocumentsReaderPaneProps {
  activePath: string;
  onSelectFolder: (path: string) => void;
  getChildren: (prefix: string | null) => PathChildEntry[];
  pathsLoading: boolean;
}

export function DocumentsReaderPane({
  activePath,
  onSelectFolder,
  getChildren,
  pathsLoading,
}: DocumentsReaderPaneProps) {
  const isRoot = activePath === ROOT_PATH;
  const prefix = isRoot ? null : activePath;

  const children = useMemo(() => getChildren(prefix), [getChildren, prefix]);
  const subfolders = useMemo(() => children.filter(isFolderEntry), [children]);
  const leafDocChildren = useMemo(() => children.filter(isLeafDocumentEntry), [children]);

  const breadcrumbsForUp = useMemo(() => pathBreadcrumbs(activePath), [activePath]);
  const parentCrumb = breadcrumbsForUp[breadcrumbsForUp.length - 2];
  const parentPath = parentCrumb?.path ?? ROOT_PATH;
  const parentLabel = parentCrumb?.name ?? "/";

  const leafDocIds = useMemo(
    () => leafDocChildren.map((c) => c.document?.document_id).filter((id): id is string => !!id),
    [leafDocChildren],
  );
  const { data: leafDocs, isLoading: leafDocsLoading } = useDocumentsByIds(leafDocIds);
  const { data: rootDocs, isLoading: rootDocsLoading } = useUncategorizedDocuments(isRoot);

  const docs: DocumentSummaryRecord[] = useMemo(() => {
    if (isRoot) {
      return (rootDocs?.documents ?? []).filter((d) => !d.document.archived);
    }
    return leafDocs;
  }, [isRoot, rootDocs, leafDocs]);

  const breadcrumbs = pathBreadcrumbs(activePath);
  const isLoading = isRoot ? rootDocsLoading : pathsLoading || leafDocsLoading;
  const totalFolders = subfolders.length;
  const totalFiles = docs.length;

  return (
    <div className={styles.pane} data-testid="documents-reader-pane">
      <div className={styles.breadcrumb}>
        {breadcrumbs.map((b, i) => {
          const isLast = i === breadcrumbs.length - 1;
          return (
            <span key={b.path}>
              {i > 0 && <span className={styles.crumbSep}>/</span>}
              <span
                className={isLast ? styles.crumbCurrent : styles.crumb}
                onClick={isLast ? undefined : () => onSelectFolder(b.path)}
              >
                {b.name}
              </span>
            </span>
          );
        })}
        <span className={styles.crumbSpacer} />
        <span className={styles.crumbMeta}>
          {totalFiles} {totalFiles === 1 ? "file" : "files"} · {totalFolders}{" "}
          {totalFolders === 1 ? "folder" : "folders"}
        </span>
      </div>

      <div className={styles.paneBody}>
        {isLoading && totalFiles === 0 && totalFolders === 0 && (
          <div className={styles.center}>
            <Spinner size="md" />
          </div>
        )}

        {!isRoot && (
          <div
            className={`${styles.docRow} ${styles.docRowUp}`}
            data-testid="documents-up-one-level"
            onClick={() => onSelectFolder(parentPath)}
            role="button"
            tabIndex={0}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                onSelectFolder(parentPath);
              }
            }}
          >
            <span className={`${styles.docRowIcon} ${styles.docRowUpIcon}`}>
              <Icons.IconChevronRight size={14} />
            </span>
            <span className={styles.docRowTitle}>Up to {parentLabel}</span>
          </div>
        )}

        {!isLoading && totalFiles === 0 && totalFolders === 0 && (
          <div className={styles.empty}>This folder is empty.</div>
        )}

        {subfolders.map((f) => (
          <div
            key={f.full_path}
            className={styles.docRow}
            onClick={() => onSelectFolder(f.full_path)}
            role="button"
            tabIndex={0}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                onSelectFolder(f.full_path);
              }
            }}
          >
            <span className={styles.docRowIcon}>
              <Icons.IconFolder size={14} />
            </span>
            <span className={styles.docRowTitle}>{f.name}</span>
            <span className={styles.docRowMeta}>
              {Number(f.child_count)} {Number(f.child_count) === 1 ? "file" : "files"}
            </span>
          </div>
        ))}

        {docs.map((doc) => (
          <Link
            key={doc.document_id}
            to={`/documents/${doc.document_id}`}
            className={styles.docRow}
          >
            <span className={styles.docRowIcon}>
              <Icons.IconDoc size={14} />
            </span>
            <span className={styles.docRowTitle}>{getDocumentDisplayTitle(doc)}</span>
            <span className={styles.docRowDate}>
              <AgoTime iso={doc.timestamp} />
            </span>
          </Link>
        ))}
      </div>
    </div>
  );
}
