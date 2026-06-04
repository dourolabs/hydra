import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

/**
 * The firing history for a trigger comes from the relations graph: each
 * `Trigger -created-> X` edge represents one successful fire. Replaces the
 * version log for activity tracking — versions only change on user edits.
 */
export function useTriggerFiringHistory(triggerId: string) {
  return useQuery({
    queryKey: ["trigger-firing-history", triggerId],
    queryFn: () =>
      apiClient.listRelations({
        source_id: triggerId,
        rel_type: "created",
      }),
    enabled: !!triggerId,
    select: (data) =>
      [...data.relations].sort((a, b) =>
        a.created_at < b.created_at ? 1 : a.created_at > b.created_at ? -1 : 0,
      ),
  });
}
