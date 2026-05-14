import { useQuery } from "@tanstack/react-query";
import type {
  ConversationSummary,
  DocumentSummaryRecord,
  IssueSummaryRecord,
  PatchSummaryRecord,
  SessionSummaryRecord,
} from "@hydra/api";
import { apiClient } from "../../api/client";

const PER_TYPE_LIMIT = 5;

export interface GlobalSearchResults {
  query: string;
  enabled: boolean;
  isLoading: boolean;
  issues: IssueSummaryRecord[];
  patches: PatchSummaryRecord[];
  documents: DocumentSummaryRecord[];
  conversations: ConversationSummary[];
  sessions: SessionSummaryRecord[];
}

export function useGlobalSearch(query: string): GlobalSearchResults {
  const enabled = query.length > 0;

  const issues = useQuery({
    queryKey: ["global-search", "issues", query],
    queryFn: () => apiClient.listIssues({ q: query, limit: PER_TYPE_LIMIT }),
    enabled,
  });

  const patches = useQuery({
    queryKey: ["global-search", "patches", query],
    queryFn: () => apiClient.listPatches({ q: query, limit: PER_TYPE_LIMIT }),
    enabled,
  });

  const documents = useQuery({
    queryKey: ["global-search", "documents", query],
    queryFn: () =>
      apiClient.listDocuments({ q: query, limit: PER_TYPE_LIMIT }),
    enabled,
  });

  const conversations = useQuery({
    queryKey: ["global-search", "conversations", query],
    queryFn: () =>
      apiClient.listConversations({ q: query, limit: PER_TYPE_LIMIT }),
    enabled,
  });

  const sessions = useQuery({
    queryKey: ["global-search", "sessions", query],
    queryFn: () => apiClient.listSessions({ q: query, limit: PER_TYPE_LIMIT }),
    enabled,
  });

  return {
    query,
    enabled,
    isLoading:
      enabled &&
      (issues.isLoading ||
        patches.isLoading ||
        documents.isLoading ||
        conversations.isLoading ||
        sessions.isLoading),
    issues: issues.data?.issues ?? [],
    patches: patches.data?.patches ?? [],
    documents: documents.data?.documents ?? [],
    conversations: conversations.data ?? [],
    sessions: sessions.data?.sessions ?? [],
  };
}
