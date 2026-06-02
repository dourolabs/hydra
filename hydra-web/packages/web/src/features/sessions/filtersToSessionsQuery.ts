import type { Filter } from "../filters";
import type { SessionFilters } from "./usePaginatedSessions";

/**
 * Translate FilterBar state + free-text `q` into the `SessionFilters` query
 * shape consumed by `usePaginatedSessions` / `useSessionCount`. This is the
 * sole mapping from FilterBar → server query: the Sessions page no longer
 * narrows client-side.
 *
 * Filter shapes:
 *   - `status` is multi-select; the values are joined into a comma-separated
 *     server param (matches `SearchSessionsQuery.status: Vec<Status>`).
 *   - `creator` is single-select; the bare username (strip the
 *     `users/` / `agents/` Principal-path prefix that `useUserOptions`
 *     surfaces) is sent as `creator`.
 *   - `relatedIssue` (multi) and the relation resolver's `patchIssueIds`
 *     (from `relatedPatch`) both map to `spawned_from_ids`. When both are
 *     active they intersect (AND across the two filters); when neither is
 *     active, no `spawned_from_ids` param is sent.
 *   - `relatedChat` is single-select; the sole value is sent as
 *     `conversation_id`.
 *   - `op: "not_in"` is unsupported for every entry; such filters are dropped.
 */
export interface BuildSessionsQueryArgs {
  filters: Filter[];
  q: string;
  /**
   * Issue ids returned by the relation resolver for `relatedPatch` (the
   * 2-hop /v1/relations → has-patch lookup). `null` when no relatedPatch
   * filter is active; an empty array means "active but matched nothing".
   */
  patchIssueIds: string[] | null;
}

// A spawned_from_ids value that no real issue id will ever match, used when a
// related-issue or related-patch filter is active but resolves to no issues.
// Sending an empty CSV would be ignored by the server.
const SENTINEL_NO_MATCH = "i-__no_match__";

function stripUserPrefix(value: string): string {
  if (value.startsWith("users/")) return value.slice("users/".length);
  if (value.startsWith("agents/")) return value.slice("agents/".length);
  return value;
}

export function filtersToSessionsQuery({
  filters,
  q,
  patchIssueIds,
}: BuildSessionsQueryArgs): SessionFilters {
  const out: SessionFilters = {};
  let relatedIssueValues: string[] | null = null;
  for (const filter of filters) {
    if (filter.op !== "in") continue;
    if (filter.values.length === 0) continue;
    switch (filter.id) {
      case "status":
        out.status = filter.values.join(",");
        break;
      case "creator":
        out.creator = stripUserPrefix(filter.values[0]);
        break;
      case "relatedIssue":
        relatedIssueValues = filter.values;
        break;
      case "relatedChat":
        out.conversation_id = filter.values[0];
        break;
      // relatedPatch is resolved into `patchIssueIds` upstream; no direct
      // mapping here.
      default:
        break;
    }
  }

  // Merge relatedIssue + patchIssueIds. The two filters AND across the page,
  // so when both are active we intersect; otherwise whichever is active wins.
  const hasRelatedIssue = relatedIssueValues !== null;
  const hasRelatedPatch = patchIssueIds !== null;
  if (hasRelatedIssue && hasRelatedPatch) {
    const patchSet = new Set(patchIssueIds);
    const intersected = (relatedIssueValues ?? []).filter((id) =>
      patchSet.has(id),
    );
    out.spawned_from_ids =
      intersected.length > 0 ? intersected.join(",") : SENTINEL_NO_MATCH;
  } else if (hasRelatedIssue) {
    out.spawned_from_ids = (relatedIssueValues ?? []).join(",");
  } else if (hasRelatedPatch) {
    out.spawned_from_ids =
      (patchIssueIds ?? []).length > 0
        ? (patchIssueIds ?? []).join(",")
        : SENTINEL_NO_MATCH;
  }

  if (q.trim()) {
    out.q = q.trim();
  }
  return out;
}
