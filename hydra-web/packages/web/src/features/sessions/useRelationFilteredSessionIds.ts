import { useMemo } from "react";
import { useQueries } from "@tanstack/react-query";
import { apiClient } from "../../api/client";
import type { Filter } from "../filters";

/**
 * Resolve the active `relatedPatch` filter into the set of *issue* ids that
 * `usePaginatedSessions` should narrow on via `spawned_from_ids`. Unlike
 * Issues' relation hook, this resolver does NOT return session ids: there is
 * no `ids[]` param on `SearchSessionsQuery`, so the 2-hop instead surfaces
 * the issues that own the selected patches (sessions are then joined by
 * `spawned_from`).
 *
 * Why only `relatedPatch` here:
 *   - `relatedIssue` maps directly to `spawned_from_ids` (no /v1/relations
 *     hop needed), so `filtersToSessionsQuery` handles it inline.
 *   - `relatedChat`  maps directly to `conversation_id` (single-valued, no
 *     hop). Also handled inline.
 *
 * Output:
 *   - `patchIssueIds: null` — no `relatedPatch` filter is active. The caller
 *     leaves the related-patch dimension out of the query.
 *   - `patchIssueIds: string[]` — issues whose `has-patch` edge points at
 *     one of the selected patches. May be empty (filter active but matched
 *     nothing); the caller maps that case to a sentinel `spawned_from_ids`.
 *   - `isLoading: true` — at least one /v1/relations request is in flight.
 */
export const SESSION_RELATION_FILTER_IDS = ["relatedPatch"] as const;

async function fetchPatchIssueIds(filter: Filter): Promise<Set<string>> {
  const issueIds = new Set<string>();
  const resp = await apiClient.listRelations({
    target_ids: filter.values.join(","),
    rel_type: "has-patch",
  });
  for (const rel of resp.relations) {
    issueIds.add(rel.source_id);
  }
  return issueIds;
}

interface ResolverPlan {
  filter: Filter;
}

function planRelationQueries(filters: Filter[]): ResolverPlan[] {
  const plans: ResolverPlan[] = [];
  for (const filter of filters) {
    if (filter.id !== "relatedPatch") continue;
    if (filter.values.length === 0) continue;
    if (filter.op !== "in") continue;
    plans.push({ filter });
  }
  return plans;
}

export interface SessionRelationResolution {
  /** Issue ids resolved from `relatedPatch`, or `null` when not active. */
  patchIssueIds: string[] | null;
  isLoading: boolean;
}

export function useRelationFilteredSessionIds(
  filters: Filter[],
): SessionRelationResolution {
  const plans = useMemo(() => planRelationQueries(filters), [filters]);

  const queries = useQueries({
    queries: plans.map((plan) => ({
      queryKey: [
        "session-relation-filter",
        plan.filter.id,
        [...plan.filter.values].sort().join(","),
      ],
      queryFn: () => fetchPatchIssueIds(plan.filter),
      staleTime: 30_000,
    })),
  });

  return useMemo<SessionRelationResolution>(() => {
    if (plans.length === 0) {
      return { patchIssueIds: null, isLoading: false };
    }
    const isLoading = queries.some((q) => q.isLoading);
    if (isLoading) {
      return { patchIssueIds: null, isLoading: true };
    }
    // AND across (currently only) the relatedPatch filter — kept as set
    // intersection so adding future per-source relations later is mechanical.
    const sets = queries.map(
      (q) => (q.data as Set<string> | undefined) ?? new Set<string>(),
    );
    if (sets.length === 0) {
      return { patchIssueIds: [], isLoading: false };
    }
    let intersected = new Set<string>(sets[0]);
    for (let i = 1; i < sets.length; i += 1) {
      const other = sets[i];
      intersected = new Set([...intersected].filter((id) => other.has(id)));
    }
    return {
      patchIssueIds: [...intersected],
      isLoading: false,
    };
  }, [plans, queries]);
}
