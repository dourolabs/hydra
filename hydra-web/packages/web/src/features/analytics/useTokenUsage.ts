import { useQuery, type UseQueryResult } from "@tanstack/react-query";
import type {
  TokenUsageCostPerAgentResponse,
  TokenUsageOverTimeQuery,
  TokenUsageOverTimeResponse,
  TokenUsageQuery,
  TokenUsageTopIssuesByCostResponse,
} from "@hydra/api";
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

export function useTokenUsageCostPerAgent(
  query: TokenUsageQuery,
  enabled: boolean = true,
): UseQueryResult<TokenUsageCostPerAgentResponse> {
  return useQuery({
    queryKey: ["analytics", "token_usage", "cost_per_agent", query],
    queryFn: () => apiClient.getTokenUsageCostPerAgent(query),
    enabled,
  });
}

export function useTokenUsageTopIssuesByCost(
  query: TokenUsageQuery,
  enabled: boolean = true,
): UseQueryResult<TokenUsageTopIssuesByCostResponse> {
  return useQuery({
    queryKey: ["analytics", "token_usage", "top_issues_by_cost", query],
    queryFn: () => apiClient.getTokenUsageTopIssuesByCost(query),
    enabled,
  });
}
