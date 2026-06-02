import { useMemo } from "react";
import { useQueries } from "@tanstack/react-query";
import { apiClient } from "../../api/client";
import type { Filter } from "../filters";

/**
 * Resolve the active relation filters into the set of patch ids the server
 * query should narrow to. One or more lookups fire per relation filter; the
 * resulting patch-id sets are intersected so combined filters read as AND
 * across the page.
 *
 * Output shape:
 *   - `patchIds: null` — no relation filter is active; the caller leaves
 *     `ids=` off the patches query entirely (no narrowing).
 *   - `patchIds: string[]` — the union of relation-matched patch ids across
 *     each filter's selected entities, intersected across filters. May be
 *     empty (relation filter active but matched nothing); the caller passes
 *     a sentinel to force a zero-row response.
 *   - `isLoading: true` — at least one relation query is still in flight.
 *
 * Relation filters and the edges they traverse:
 *   - `relatedIssue`   → `/v1/relations` source_ids=<issue_ids>,
 *                        rel_type=has-patch; collect target_ids (patch ids).
 *   - `relatedSession` → `getSession(id)` → `spawned_from` issue, then
 *                        `/v1/relations` source_ids=<issue_ids>,
 *                        rel_type=has-patch; collect target_ids. Sessions
 *                        point at their spawning issue via
 *                        `Session.spawned_from`, then the has-patch edges
 *                        give us the patches.
 */
export const RELATION_FILTER_IDS = ["relatedIssue", "relatedSession"];

async function fetchPatchIdsForRelatedIssue(
  filter: Filter,
): Promise<Set<string>> {
  const valueParam = filter.values.join(",");
  const resp = await apiClient.listRelations({
    source_ids: valueParam,
    rel_type: "has-patch",
  });
  const out = new Set<string>();
  for (const rel of resp.relations) {
    out.add(rel.target_id);
  }
  return out;
}

async function fetchPatchIdsForRelatedSession(
  filter: Filter,
): Promise<Set<string>> {
  // Hop 1: resolve each selected session to its spawning issue id.
  const sessions = await Promise.all(
    filter.values.map((id) =>
      apiClient.getSession(id).catch(() => null),
    ),
  );
  const issueIds = new Set<string>();
  for (const session of sessions) {
    const spawnedFrom = session?.session.spawned_from;
    if (spawnedFrom) issueIds.add(spawnedFrom);
  }
  if (issueIds.size === 0) return new Set<string>();

  // Hop 2: collect patches attached to those issues via the has-patch edge.
  const resp = await apiClient.listRelations({
    source_ids: [...issueIds].join(","),
    rel_type: "has-patch",
  });
  const out = new Set<string>();
  for (const rel of resp.relations) {
    out.add(rel.target_id);
  }
  return out;
}

async function fetchPatchIdsForFilter(filter: Filter): Promise<Set<string>> {
  if (filter.id === "relatedIssue") {
    return fetchPatchIdsForRelatedIssue(filter);
  }
  if (filter.id === "relatedSession") {
    return fetchPatchIdsForRelatedSession(filter);
  }
  return new Set<string>();
}

interface ResolverPlan {
  filter: Filter;
}

function planRelationQueries(filters: Filter[]): ResolverPlan[] {
  const plans: ResolverPlan[] = [];
  for (const filter of filters) {
    if (!RELATION_FILTER_IDS.includes(filter.id)) continue;
    if (filter.values.length === 0) continue;
    if (filter.op !== "in") continue;
    plans.push({ filter });
  }
  return plans;
}

export interface PatchRelationResolution {
  /** Resolved patch ids, or `null` when no relation filter is active. */
  patchIds: string[] | null;
  isLoading: boolean;
}

export function useRelationFilteredPatchIds(
  filters: Filter[],
): PatchRelationResolution {
  const plans = useMemo(() => planRelationQueries(filters), [filters]);

  const queries = useQueries({
    queries: plans.map((plan) => ({
      queryKey: [
        "patch-relation-filter",
        plan.filter.id,
        [...plan.filter.values].sort().join(","),
      ],
      queryFn: () => fetchPatchIdsForFilter(plan.filter),
      staleTime: 30_000,
    })),
  });

  return useMemo<PatchRelationResolution>(() => {
    if (plans.length === 0) {
      return { patchIds: null, isLoading: false };
    }
    const isLoading = queries.some((q) => q.isLoading);
    if (isLoading) {
      return { patchIds: null, isLoading: true };
    }
    const sets = queries.map(
      (q) => (q.data as Set<string> | undefined) ?? new Set<string>(),
    );
    if (sets.length === 0) {
      return { patchIds: [], isLoading: false };
    }
    let intersected = new Set<string>(sets[0]);
    for (let i = 1; i < sets.length; i += 1) {
      const other = sets[i];
      intersected = new Set([...intersected].filter((id) => other.has(id)));
    }
    return {
      patchIds: [...intersected],
      isLoading: false,
    };
  }, [plans, queries]);
}
