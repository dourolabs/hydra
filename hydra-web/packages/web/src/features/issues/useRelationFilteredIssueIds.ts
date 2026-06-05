import { useMemo } from "react";
import { useQueries } from "@tanstack/react-query";
import { hydraIdKind } from "@hydra/api";
import { apiClient } from "../../api/client";
import type { Filter } from "../filters";

/**
 * Resolve the active relation filters into the set of issue ids the server
 * query should narrow to. One `/v1/relations` request fires per relation
 * filter; the resulting issue-id sets are intersected so combined filters
 * read as AND across the page.
 *
 * Output shape:
 *   - `issueIds: null` — no relation filter is active; the caller leaves
 *     `ids=` off the issues query entirely (no narrowing).
 *   - `issueIds: string[]` — the union of relation-matched issue ids across
 *     each filter's selected entities, intersected across filters. May be
 *     empty (relation filter active but matched nothing); the caller passes
 *     a sentinel to force a zero-row response.
 *   - `isLoading: true` — at least one relation query is still in flight;
 *     the caller must hold off issuing `listIssues` until the resolver lands.
 *
 * Relation filters and the edges they traverse:
 *   - `relatedPatch`  → `has-patch`  outbound  (issue → patch)
 *   - `relatedChat`   → `refers-to`  inbound   (chat → issue)
 *   - `parentOrChild` → `child-of`   outbound + inbound (parent ↔ child)
 *   - `relatedSession` does NOT use a relation row; sessions point at their
 *     spawning issue via `Session.spawned_from`. Resolved here by listing
 *     sessions whose ids are the selected values and bucketing by their
 *     `spawned_from`. Mirrors how the brief framed session linkage.
 */
interface RelationQuerySpec {
  filterId: string;
  // For `/v1/relations`:
  rel_type?: string;
  direction?: "outbound" | "inbound";
  // For session lookup: list sessions by id, take their `spawned_from`.
  via?: "sessions";
}

const RELATION_QUERY_SPECS: Record<string, RelationQuerySpec[]> = {
  relatedPatch: [
    { filterId: "relatedPatch", rel_type: "has-patch", direction: "outbound" },
  ],
  relatedChat: [
    { filterId: "relatedChat", rel_type: "refers-to", direction: "inbound" },
  ],
  parentOrChild: [
    { filterId: "parentOrChild", rel_type: "child-of", direction: "outbound" },
    { filterId: "parentOrChild", rel_type: "child-of", direction: "inbound" },
  ],
  relatedSession: [{ filterId: "relatedSession", via: "sessions" }],
};

export const RELATION_FILTER_IDS = Object.keys(RELATION_QUERY_SPECS);

/**
 * Server cap on the `ids=<csv>` param accepted by `listIssues` /
 * `SearchIssuesQuery`. Exceeding it produces a 4xx (or silent truncation
 * server-side); cap the joined set client-side so the request stays valid.
 */
export const MAX_IDS_CSV_LEN = 100;

/**
 * Truncate a resolved relation-matched id set to `MAX_IDS_CSV_LEN` before it
 * goes onto the `ids=<csv>` param. When truncation happens, emits a
 * `console.warn` so it surfaces in devtools without imposing UI cost on the
 * (rare) overflow path — picker option-lists are already bounded to 100 per
 * entity type upstream.
 */
export function capRelationIds(ids: string[]): string[] {
  if (ids.length <= MAX_IDS_CSV_LEN) return ids;
  console.warn(
    `useRelationFilteredIssueIds: capping relation-matched id set ${ids.length} → ${MAX_IDS_CSV_LEN}; results past the first ${MAX_IDS_CSV_LEN} will be omitted from this page.`,
  );
  return ids.slice(0, MAX_IDS_CSV_LEN);
}

interface ResolverPlan {
  filter: Filter;
  specs: RelationQuerySpec[];
}

function planRelationQueries(filters: Filter[]): ResolverPlan[] {
  const plans: ResolverPlan[] = [];
  for (const filter of filters) {
    const specs = RELATION_QUERY_SPECS[filter.id];
    if (!specs) continue;
    if (filter.values.length === 0) continue;
    if (filter.op !== "in") continue;
    plans.push({ filter, specs });
  }
  return plans;
}

async function fetchIssueIdsForFilter(
  filter: Filter,
  specs: RelationQuerySpec[],
): Promise<Set<string>> {
  const valueParam = filter.values.join(",");
  const issueIds = new Set<string>();
  for (const spec of specs) {
    if (spec.via === "sessions") {
      // Session relations ride on `Session.spawned_from`, not a relation row.
      // Resolve each selected session to its spawning issue via the per-id
      // get endpoint; sessions selected from the picker are bounded (handful
      // typically), so a small Promise.all batch keeps latency in check.
      const sessions = await Promise.all(
        filter.values.map((id) =>
          apiClient.getSession(id).catch(() => null),
        ),
      );
      for (const session of sessions) {
        const spawnedFrom = session?.session.spawned_from;
        if (spawnedFrom && hydraIdKind(spawnedFrom) === "issue") {
          issueIds.add(spawnedFrom);
        }
      }
      continue;
    }
    if (!spec.rel_type || !spec.direction) continue;
    const params =
      spec.direction === "outbound"
        ? { target_ids: valueParam, rel_type: spec.rel_type }
        : { source_ids: valueParam, rel_type: spec.rel_type };
    const resp = await apiClient.listRelations(params);
    for (const rel of resp.relations) {
      // Outbound: issue is source, target is the related entity → walk
      //   source ← target (filter selected target_ids, collect source_ids).
      // Inbound: issue is target, source is the related entity → walk
      //   target ← source (filter selected source_ids, collect target_ids).
      const candidateId =
        spec.direction === "outbound" ? rel.source_id : rel.target_id;
      // Conversation→artifact `refers-to` edges fan out across all artifact
      // kinds (issues, patches, documents), so an inbound walk may surface
      // non-`i-` ids. The CSV is forwarded to `/v1/issues?ids=`, which the
      // real backend deserializes as `Vec<IssueId>` — a single non-issue id
      // rejects the whole query with 400. Other specs (has-patch outbound,
      // child-of, session spawned_from) naturally only yield issue ids in
      // the field we read here, so this bucket is a safety net there too.
      if (hydraIdKind(candidateId) !== "issue") continue;
      issueIds.add(candidateId);
    }
  }
  return issueIds;
}

export interface RelationResolution {
  /** Resolved issue ids, or `null` when no relation filter is active. */
  issueIds: string[] | null;
  isLoading: boolean;
}

export function useRelationFilteredIssueIds(
  filters: Filter[],
): RelationResolution {
  const plans = useMemo(() => planRelationQueries(filters), [filters]);

  const queries = useQueries({
    queries: plans.map((plan) => ({
      queryKey: [
        "issue-relation-filter",
        plan.filter.id,
        [...plan.filter.values].sort().join(","),
      ],
      queryFn: () => fetchIssueIdsForFilter(plan.filter, plan.specs),
      staleTime: 30_000,
    })),
  });

  return useMemo<RelationResolution>(() => {
    if (plans.length === 0) {
      return { issueIds: null, isLoading: false };
    }
    const isLoading = queries.some((q) => q.isLoading);
    if (isLoading) {
      return { issueIds: null, isLoading: true };
    }
    // AND across filters: intersect each filter's matched issue id set.
    const sets = queries.map(
      (q) => (q.data as Set<string> | undefined) ?? new Set<string>(),
    );
    if (sets.length === 0) {
      return { issueIds: [], isLoading: false };
    }
    let intersected = new Set<string>(sets[0]);
    for (let i = 1; i < sets.length; i += 1) {
      const other = sets[i];
      intersected = new Set([...intersected].filter((id) => other.has(id)));
    }
    return {
      issueIds: capRelationIds([...intersected]),
      isLoading: false,
    };
  }, [plans, queries]);
}
