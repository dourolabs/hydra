import type { DocumentSummaryRecord } from "@hydra/api";

export function getDocumentDisplayTitle(doc: DocumentSummaryRecord): string {
  if (doc.document.title) return doc.document.title;
  if (doc.document.path) return doc.document.path;
  return doc.document_id;
}
