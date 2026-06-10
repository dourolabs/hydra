import { useQuery, type UseQueryResult } from "@tanstack/react-query";
import type {
  PatchesOverTimeQuery,
  PatchesOverTimeResponse,
  PatchesTerminalMixQuery,
  PatchesTerminalMixResponse,
  PatchesTimeToMergeQuery,
  PatchesTimeToMergeResponse,
  PatchesInFlightOverTimeQuery,
  PatchesInFlightOverTimeResponse,
} from "@hydra/api";
import { apiClient } from "../../api/client";

/**
 * Throughput / patches data hooks. Each wraps a single backend endpoint
 * with React Query. Query keys serialize the full param object so a slicer
 * change reliably invalidates the previous request.
 */

export function useThroughputPatchesOverTime(
  query: PatchesOverTimeQuery,
  enabled: boolean = true,
): UseQueryResult<PatchesOverTimeResponse> {
  return useQuery({
    queryKey: ["analytics", "throughput", "patches", "over_time", query],
    queryFn: () => apiClient.getPatchesThroughputOverTime(query),
    enabled,
  });
}

export function useThroughputPatchesTerminalMix(
  query: PatchesTerminalMixQuery,
  enabled: boolean = true,
): UseQueryResult<PatchesTerminalMixResponse> {
  return useQuery({
    queryKey: ["analytics", "throughput", "patches", "terminal_mix", query],
    queryFn: () => apiClient.getPatchesThroughputTerminalMix(query),
    enabled,
  });
}

export function useThroughputPatchesTimeToMerge(
  query: PatchesTimeToMergeQuery,
  enabled: boolean = true,
): UseQueryResult<PatchesTimeToMergeResponse> {
  return useQuery({
    queryKey: ["analytics", "throughput", "patches", "time_to_merge", query],
    queryFn: () => apiClient.getPatchesThroughputTimeToMerge(query),
    enabled,
  });
}

export function useThroughputPatchesInFlightOverTime(
  query: PatchesInFlightOverTimeQuery,
  enabled: boolean = true,
): UseQueryResult<PatchesInFlightOverTimeResponse> {
  return useQuery({
    queryKey: ["analytics", "throughput", "patches", "in_flight_over_time", query],
    queryFn: () => apiClient.getPatchesThroughputInFlightOverTime(query),
    enabled,
  });
}
