import { useQuery, type UseQueryResult } from "@tanstack/react-query";
import type { TokenUsageOverTimeQuery, TokenUsageOverTimeResponse } from "@hydra/api";
import { apiClient } from "../../api/client";

/**
 * Token-usage data hooks. Each wraps a single backend endpoint with React
 * Query; the full param object is serialized into the query key so a
 * time-range change reliably invalidates the previous request.
 */

export function useTokenUsageOverTime(
  query: TokenUsageOverTimeQuery,
  enabled: boolean = true,
): UseQueryResult<TokenUsageOverTimeResponse> {
  return useQuery({
    queryKey: ["analytics", "token_usage", "over_time", query],
    queryFn: () => apiClient.getTokenUsageOverTime(query),
    enabled,
  });
}
