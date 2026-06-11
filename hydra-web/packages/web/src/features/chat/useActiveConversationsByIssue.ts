import { useMemo } from "react";
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import type { ConversationSummary } from "@hydra/api";
import { apiClient } from "../../api/client";

/**
 * For each issue id, fetch the conversations spawned from it and return the
 * most-recently-updated non-closed one (active or idle) keyed by issue id.
 * Used by the board / table views to flag issues that have a live conversation
 * a human can join.
 *
 * Issues the request as a single batched call via
 * `SearchConversationsQuery.spawned_from_ids`, mirroring the sessions pattern
 * in `usePageIssueTrees`.
 */
export function useActiveConversationsByIssue(
  issueIds: string[],
): Map<string, ConversationSummary> {
  const spawned_from_ids = useMemo(
    () => [...issueIds].sort().join(","),
    [issueIds],
  );

  const { data } = useQuery({
    queryKey: ["conversations", "batch", spawned_from_ids],
    queryFn: () =>
      apiClient.listConversations({
        spawned_from_ids,
        include_deleted: false,
      }),
    enabled: issueIds.length > 0,
    staleTime: 30_000,
    placeholderData: keepPreviousData,
    select: (resp) => resp.conversations,
  });

  return useMemo(() => {
    const map = new Map<string, ConversationSummary>();
    if (!data) return map;
    for (const conv of data) {
      if (conv.status === "closed") continue;
      const issueId = conv.spawned_from;
      if (!issueId) continue;
      const existing = map.get(issueId);
      if (!existing || conv.updated_at > existing.updated_at) {
        map.set(issueId, conv);
      }
    }
    return map;
  }, [data]);
}
