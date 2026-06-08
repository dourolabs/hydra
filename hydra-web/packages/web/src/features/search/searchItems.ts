import type {
  ConversationSummary,
  DocumentSummaryRecord,
  IssueSummaryRecord,
  PatchSummaryRecord,
  SessionSummaryRecord,
} from "@hydra/api";
import { conversationTitle } from "../chat/conversationTitle";

export type SearchItemKind =
  | "issue"
  | "patch"
  | "document"
  | "conversation"
  | "session";

export interface SearchItem {
  kind: SearchItemKind;
  id: string;
  label: string;
  href: string;
  /** Short mono meta string shown on the right side of the row. */
  meta?: string;
  /** Project key for issue rows; resolved by the caller against the
   * loaded project list. Null when projects are still loading or no
   * matching project is found. */
  projectKey?: string | null;
}

function resolveProjectKey(
  projectsById: Map<string, string>,
  projectId: string | null | undefined,
): string | null {
  if (projectsById.size === 0) return null;
  if (projectId) {
    return projectsById.get(projectId) ?? null;
  }
  for (const key of projectsById.values()) {
    if (key === "default") return key;
  }
  return null;
}

export function issueToItem(
  record: IssueSummaryRecord,
  projectsById: Map<string, string>,
): SearchItem {
  return {
    kind: "issue",
    id: record.issue_id,
    label: record.issue.title || record.issue_id,
    href: `/issues/${record.issue_id}`,
    meta: record.issue.status.replace("-", " "),
    projectKey: resolveProjectKey(projectsById, record.issue.project_id),
  };
}

export function patchToItem(record: PatchSummaryRecord): SearchItem {
  return {
    kind: "patch",
    id: record.patch_id,
    label: record.patch.title || record.patch_id,
    href: `/patches/${record.patch_id}`,
    meta: record.patch.service_repo_name,
  };
}

export function documentToItem(record: DocumentSummaryRecord): SearchItem {
  const title = record.document.title?.trim();
  const path = record.document.path?.trim();
  return {
    kind: "document",
    id: record.document_id,
    label: title || path || record.document_id,
    href: `/documents/${record.document_id}`,
    meta: path ?? undefined,
  };
}

export function conversationToItem(c: ConversationSummary): SearchItem {
  return {
    kind: "conversation",
    id: c.conversation_id,
    label: conversationTitle(c),
    href: `/chat/${c.conversation_id}`,
    meta: c.status,
  };
}

export function sessionToItem(record: SessionSummaryRecord): SearchItem {
  const prompt = record.session.prompt?.trim();
  const label = prompt ? prompt : record.session_id;
  return {
    kind: "session",
    id: record.session_id,
    label,
    href: `/sessions/${record.session_id}`,
    meta: record.session.status,
  };
}
