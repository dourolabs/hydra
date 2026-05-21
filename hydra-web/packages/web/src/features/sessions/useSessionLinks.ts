import { keepPreviousData, useQueries, useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import type {
  Conversation,
  IssueSummaryRecord,
  SessionSummaryRecord,
} from "@hydra/api";
import { apiClient } from "../../api/client";

export interface SessionLinks {
  issueMap: Map<string, IssueSummaryRecord>;
  conversationMap: Map<string, Conversation>;
}

/**
 * Resolve the linked issue (via `spawned_from`) and conversation (via
 * `conversation_id`) for each session in a list, so callers can render titles
 * and assignees from the linked entity instead of the raw session prompt.
 *
 * Issues are batch-fetched in a single `listIssues({ ids })` call;
 * conversations are fetched one at a time (no batch endpoint) but cached
 * under the same `["conversation", id]` key used by `useConversation`.
 */
export function useSessionLinks(records: SessionSummaryRecord[]): SessionLinks {
  const issueIds = useMemo(() => {
    const set = new Set<string>();
    for (const r of records) {
      if (r.session.spawned_from) set.add(r.session.spawned_from);
    }
    return [...set].sort();
  }, [records]);

  const conversationIds = useMemo(() => {
    const set = new Set<string>();
    for (const r of records) {
      if (r.session.conversation_id) set.add(r.session.conversation_id);
    }
    return [...set].sort();
  }, [records]);

  const idsParam = issueIds.join(",");
  const { data: issues } = useQuery({
    queryKey: ["issues", "batch", idsParam],
    queryFn: () => apiClient.listIssues({ ids: idsParam }),
    enabled: issueIds.length > 0,
    staleTime: 30_000,
    placeholderData: keepPreviousData,
    select: (data) => data.issues,
  });

  const conversationQueries = useQueries({
    queries: conversationIds.map((id) => ({
      queryKey: ["conversation", id],
      queryFn: () => apiClient.getConversation(id),
      staleTime: 30_000,
      enabled: !!id,
    })),
  });

  const issueMap = useMemo(() => {
    const map = new Map<string, IssueSummaryRecord>();
    for (const issue of issues ?? []) map.set(issue.issue_id, issue);
    return map;
  }, [issues]);

  const conversationMap = useMemo(() => {
    const map = new Map<string, Conversation>();
    for (const q of conversationQueries) {
      if (q.data) map.set(q.data.conversation_id, q.data);
    }
    return map;
  }, [conversationQueries]);

  return { issueMap, conversationMap };
}
