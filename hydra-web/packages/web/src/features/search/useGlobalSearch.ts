import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type {
  ConversationSummary,
  DocumentSummaryRecord,
  IssueSummaryRecord,
  PatchSummaryRecord,
  SessionSummaryRecord,
} from "@hydra/api";
import { apiClient } from "../../api/client";
import { conversationTitle } from "../chat/conversationTitle";

export type GlobalSearchRowType =
  | "issue"
  | "patch"
  | "document"
  | "conversation"
  | "session";

export interface GlobalSearchRow {
  type: GlobalSearchRowType;
  id: string;
  label: string;
  /** Destination route. `null` means the row is not navigable (rendered as plain text). */
  to: string | null;
}

export interface GlobalSearchGroup {
  type: GlobalSearchRowType;
  label: string;
  rows: GlobalSearchRow[];
}

const PER_TYPE_LIMIT = 5;

export const GROUP_LABELS: Record<GlobalSearchRowType, string> = {
  issue: "Issues",
  patch: "Patches",
  document: "Documents",
  conversation: "Chats",
  session: "Sessions",
};

/** The group display order. */
export const GROUP_ORDER: readonly GlobalSearchRowType[] = [
  "issue",
  "patch",
  "document",
  "conversation",
  "session",
];

export function useDebouncedValue<T>(value: T, delayMs: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const handle = setTimeout(() => setDebounced(value), delayMs);
    return () => clearTimeout(handle);
  }, [value, delayMs]);
  return debounced;
}

function issueRow(record: IssueSummaryRecord): GlobalSearchRow {
  return {
    type: "issue",
    id: record.issue_id,
    label: record.issue.title || record.issue_id,
    to: `/issues/${record.issue_id}`,
  };
}

function patchRow(record: PatchSummaryRecord): GlobalSearchRow {
  return {
    type: "patch",
    id: record.patch_id,
    label: record.patch.title || record.patch_id,
    to: `/patches/${record.patch_id}`,
  };
}

function documentRow(record: DocumentSummaryRecord): GlobalSearchRow {
  const title = record.document.title || record.document.path || record.document_id;
  return {
    type: "document",
    id: record.document_id,
    label: title,
    to: `/documents/${record.document_id}`,
  };
}

function conversationRow(c: ConversationSummary): GlobalSearchRow {
  return {
    type: "conversation",
    id: c.conversation_id,
    label: conversationTitle(c),
    to: `/chat/${c.conversation_id}`,
  };
}

function sessionRow(record: SessionSummaryRecord): GlobalSearchRow {
  const spawned = record.session.spawned_from;
  const promptPrefix = record.session.prompt
    ? record.session.prompt.trim()
    : "";
  const label = promptPrefix.length > 0 ? promptPrefix : record.session_id;
  const to =
    typeof spawned === "string" && spawned.length > 0
      ? `/issues/${spawned}/sessions/${record.session_id}/logs`
      : null;
  return {
    type: "session",
    id: record.session_id,
    label,
    to,
  };
}

export interface UseGlobalSearchResult {
  debouncedQuery: string;
  groups: GlobalSearchGroup[];
  /** Flat list of rows in group order — used for keyboard navigation. */
  flatRows: GlobalSearchRow[];
  isLoading: boolean;
}

/**
 * Fan out a single text query across the 5 list endpoints and group the
 * results by type. Queries are skipped while the (debounced) query is empty.
 */
export function useGlobalSearch(
  rawQuery: string,
  debounceMs = 200,
): UseGlobalSearchResult {
  const debouncedQuery = useDebouncedValue(rawQuery.trim(), debounceMs);
  const enabled = debouncedQuery.length > 0;

  const issuesQuery = useQuery({
    queryKey: ["global-search", "issue", debouncedQuery],
    queryFn: () =>
      apiClient.listIssues({ q: debouncedQuery, limit: PER_TYPE_LIMIT }),
    enabled,
  });
  const patchesQuery = useQuery({
    queryKey: ["global-search", "patch", debouncedQuery],
    queryFn: () =>
      apiClient.listPatches({ q: debouncedQuery, limit: PER_TYPE_LIMIT }),
    enabled,
  });
  const documentsQuery = useQuery({
    queryKey: ["global-search", "document", debouncedQuery],
    queryFn: () =>
      apiClient.listDocuments({ q: debouncedQuery, limit: PER_TYPE_LIMIT }),
    enabled,
  });
  const conversationsQuery = useQuery({
    queryKey: ["global-search", "conversation", debouncedQuery],
    queryFn: () =>
      apiClient.listConversations({ q: debouncedQuery, limit: PER_TYPE_LIMIT }),
    enabled,
  });
  const sessionsQuery = useQuery({
    queryKey: ["global-search", "session", debouncedQuery],
    queryFn: () =>
      apiClient.listSessions({ q: debouncedQuery, limit: PER_TYPE_LIMIT }),
    enabled,
  });

  const issueRows: GlobalSearchRow[] =
    issuesQuery.data?.issues.map(issueRow) ?? [];
  const patchRows: GlobalSearchRow[] =
    patchesQuery.data?.patches.map(patchRow) ?? [];
  const documentRows: GlobalSearchRow[] =
    documentsQuery.data?.documents.map(documentRow) ?? [];
  const conversationRows: GlobalSearchRow[] =
    conversationsQuery.data?.map(conversationRow) ?? [];
  const sessionRows: GlobalSearchRow[] =
    sessionsQuery.data?.sessions.map(sessionRow) ?? [];

  const groups: GlobalSearchGroup[] = [
    { type: "issue", label: GROUP_LABELS.issue, rows: issueRows },
    { type: "patch", label: GROUP_LABELS.patch, rows: patchRows },
    { type: "document", label: GROUP_LABELS.document, rows: documentRows },
    { type: "conversation", label: GROUP_LABELS.conversation, rows: conversationRows },
    { type: "session", label: GROUP_LABELS.session, rows: sessionRows },
  ];

  const flatRows = groups.flatMap((g) => g.rows);

  const isLoading =
    enabled &&
    (issuesQuery.isLoading ||
      patchesQuery.isLoading ||
      documentsQuery.isLoading ||
      conversationsQuery.isLoading ||
      sessionsQuery.isLoading);

  return { debouncedQuery, groups, flatRows, isLoading };
}
