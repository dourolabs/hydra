import { useMemo } from "react";
import { useQueries } from "@tanstack/react-query";
import type { ConversationSummary } from "@hydra/api";
import { apiClient } from "../../api/client";

/**
 * For each issue id, fetch the conversations spawned from it and return the
 * most-recently-updated non-closed one (active or idle) keyed by issue id.
 * Used by the board / table views to flag issues that have a live conversation
 * a human can join.
 *
 * The API has no batch-by-issue-id filter for conversations, so this fires
 * one request per id and lets React Query handle parallelization, caching,
 * and dedup. SSE invalidation on `["conversations", …]` keeps each entry
 * fresh.
 */
export function useActiveConversationsByIssue(
  issueIds: string[],
): Map<string, ConversationSummary> {
  const sortedIds = useMemo(() => [...issueIds].sort(), [issueIds]);

  const results = useQueries({
    queries: sortedIds.map((issueId) => ({
      queryKey: [
        "conversations",
        { spawned_from: issueId, include_deleted: false },
      ],
      queryFn: async () => {
        const resp = await apiClient.listConversations({
          spawned_from: issueId,
          include_deleted: false,
        });
        return resp.conversations;
      },
      staleTime: 30_000,
      enabled: !!issueId,
    })),
  });

  return useMemo(() => {
    const map = new Map<string, ConversationSummary>();
    sortedIds.forEach((issueId, idx) => {
      const conversations = results[idx]?.data;
      if (!conversations || conversations.length === 0) return;
      const live = conversations.find((c) => c.status !== "closed");
      if (live) map.set(issueId, live);
    });
    return map;
    // results is a new array each render but each entry is stable; depend on
    // the data references rather than the array identity.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sortedIds, ...results.map((r) => r.data)]);
}
