import { useQuery, type UseQueryResult } from "@tanstack/react-query";
import type {
  IssuesCycleTimeQuery,
  IssuesCycleTimeResponse,
  IssuesTimeInStatusBreakdownQuery,
  IssuesTimeInStatusBreakdownResponse,
  IssuesPerStatusDistributionQuery,
  IssuesPerStatusDistributionResponse,
  IssuesOverTimeQuery,
  IssuesOverTimeResponse,
} from "@hydra/api";
import { apiClient } from "../../api/client";

/**
 * Throughput / issues data hooks. Two of the endpoints
 * (`time_in_status_breakdown`, `per_status_distribution`) require a
 * `project_id` — the page is responsible for disabling those hooks when
 * the slicer Project filter is empty.
 */

export function useThroughputIssuesCycleTime(
  query: IssuesCycleTimeQuery,
  enabled: boolean = true,
): UseQueryResult<IssuesCycleTimeResponse> {
  return useQuery({
    queryKey: ["analytics", "throughput", "issues", "cycle_time", query],
    queryFn: () => apiClient.getIssuesThroughputCycleTime(query),
    enabled,
  });
}

export function useThroughputIssuesTimeInStatusBreakdown(
  query: IssuesTimeInStatusBreakdownQuery,
  enabled: boolean = true,
): UseQueryResult<IssuesTimeInStatusBreakdownResponse> {
  return useQuery({
    queryKey: ["analytics", "throughput", "issues", "time_in_status_breakdown", query],
    queryFn: () => apiClient.getIssuesThroughputTimeInStatusBreakdown(query),
    enabled: enabled && !!query.project_id,
  });
}

export function useThroughputIssuesPerStatusDistribution(
  query: IssuesPerStatusDistributionQuery,
  enabled: boolean = true,
): UseQueryResult<IssuesPerStatusDistributionResponse> {
  return useQuery({
    queryKey: ["analytics", "throughput", "issues", "per_status_distribution", query],
    queryFn: () => apiClient.getIssuesThroughputPerStatusDistribution(query),
    enabled: enabled && !!query.project_id,
  });
}

export function useThroughputIssuesOverTime(
  query: IssuesOverTimeQuery,
  enabled: boolean = true,
): UseQueryResult<IssuesOverTimeResponse> {
  return useQuery({
    queryKey: ["analytics", "throughput", "issues", "over_time", query],
    queryFn: () => apiClient.getIssuesThroughputOverTime(query),
    enabled,
  });
}
