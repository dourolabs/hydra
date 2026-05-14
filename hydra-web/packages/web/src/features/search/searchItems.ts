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
}

export function issueToItem(record: IssueSummaryRecord): SearchItem {
  return {
    kind: "issue",
    id: record.issue_id,
    label: record.issue.title,
    href: `/issues/${record.issue_id}`,
  };
}

export function patchToItem(record: PatchSummaryRecord): SearchItem {
  return {
    kind: "patch",
    id: record.patch_id,
    label: record.patch.title,
    href: `/patches/${record.patch_id}`,
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
  };
}

export function conversationToItem(c: ConversationSummary): SearchItem {
  return {
    kind: "conversation",
    id: c.conversation_id,
    label: conversationTitle(c),
    href: `/chat/${c.conversation_id}`,
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
  };
}
