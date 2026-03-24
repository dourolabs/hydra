import type { DocumentSummaryRecord } from "@hydra/api";

export interface DocumentGroup {
  prefix: string;
  documents: DocumentSummaryRecord[];
}

function getPathPrefix(doc: DocumentSummaryRecord): string {
  const path = doc.document.path;
  if (!path) return "";
  // Strip leading slash, then take the first path segment
  const cleaned = path.startsWith("/") ? path.slice(1) : path;
  const slashIndex = cleaned.indexOf("/");
  if (slashIndex < 0) return "";
  return cleaned.slice(0, slashIndex);
}

export function groupDocumentsByPrefix(documents: DocumentSummaryRecord[]): DocumentGroup[] {
  const groups = new Map<string, DocumentSummaryRecord[]>();

  for (const doc of documents) {
    if (doc.document.deleted) continue;
    const prefix = getPathPrefix(doc);
    const list = groups.get(prefix) ?? [];
    list.push(doc);
    groups.set(prefix, list);
  }

  // Sort groups alphabetically, with uncategorized ("") last
  const sorted: DocumentGroup[] = [];
  const keys = Array.from(groups.keys()).sort((a, b) => {
    if (a === "") return 1;
    if (b === "") return -1;
    return a.localeCompare(b);
  });

  for (const key of keys) {
    const docs = groups.get(key)!;
    docs.sort((a, b) => {
      const pathA = a.document.path;
      const pathB = b.document.path;
      if (!pathA && !pathB) return 0;
      if (!pathA) return 1;
      if (!pathB) return -1;
      return pathA.localeCompare(pathB);
    });
    sorted.push({ prefix: key, documents: docs });
  }

  return sorted;
}

export function getDocumentDisplayTitle(doc: DocumentSummaryRecord): string {
  if (doc.document.title) return doc.document.title;
  if (doc.document.path) return doc.document.path;
  return doc.document_id;
}
