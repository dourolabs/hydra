import type { Filter } from "../filters";
import type { IssueFilters } from "./usePaginatedIssues";

/**
 * Translate FilterBar state + free-text `q` into the `IssueFilters` query
 * shape consumed by `usePaginatedIssues` / `useIssueCount`. This is the only
 * mapping from FilterBar → server query: the Issues page no longer narrows
 * client-side.
 *
 * Filter shapes:
 *   - `status` / `type` / `creator` / `assignee` are single-select; take the
 *     sole value as a single-value server param.
 *   - `creator` server param expects a bare username; strip the
 *     `users/` / `agents/` Principal-path prefix that `useUserOptions`
 *     surfaces.
 *   - Relation filters (`relatedPatch` / `relatedChat` / `relatedSession` /
 *     `parentOrChild`) do NOT map to a direct `listIssues` param. The caller
 *     resolves them to a set of issue ids via `/v1/relations` and passes the
 *     resolved set as `extraIds`; this mapper unions `extraIds` with any
 *     explicit `ids=` already on the query (none today, but kept for
 *     symmetry).
 *   - `op: "not_in"` is not server-applicable for any filter in PR-1; such
 *     filters are silently dropped by this mapper. (`notInSupported` defaults
 *     to false in the FilterDefinition so the UI never produces them.)
 */
export interface BuildIssuesQueryArgs {
  filters: Filter[];
  q: string;
  /**
   * Issue ids returned by the relation resolver (or `null` when no relation
   * filters are active). When provided, the resulting query uses
   * `ids=<comma-joined>` to narrow the server response — including the empty
   * case, which intentionally produces zero results.
   */
  extraIds: string[] | null;
}

function stripUserPrefix(value: string): string {
  if (value.startsWith("users/")) return value.slice("users/".length);
  if (value.startsWith("agents/")) return value.slice("agents/".length);
  return value;
}

const SENTINEL_NO_MATCH = "__no_match__";

export function filtersToIssuesQuery({
  filters,
  q,
  extraIds,
}: BuildIssuesQueryArgs): IssueFilters {
  const out: IssueFilters = {};
  for (const filter of filters) {
    if (filter.op !== "in") continue;
    if (filter.values.length === 0) continue;
    switch (filter.id) {
      case "status":
        out.status = filter.values[0];
        break;
      case "type":
        out.type = filter.values[0];
        break;
      case "creator":
        out.creator = stripUserPrefix(filter.values[0]);
        break;
      case "assignee":
        out.assignee = filter.values[0];
        break;
      case "project":
        // The backend `ProjectId` validator only accepts `j-`-prefixed
        // ids; a transiently unresolved project key (e.g. pasted
        // `?project=engineering-v2` mid-resolution) would otherwise 400.
        // The page-level resolver canonicalizes URL→state on the next
        // render; we just need to avoid emitting the bad query in the
        // meantime.
        if (filter.values[0].startsWith("j-")) {
          out.project_id = filter.values[0];
        }
        break;
      // Relation filters are resolved upstream into `extraIds`; no direct
      // mapping here.
      default:
        break;
    }
  }
  if (q.trim()) {
    out.q = q.trim();
  }
  if (extraIds !== null) {
    if (extraIds.length === 0) {
      // The relation filter is active but matched no issues. We can't pass an
      // empty `ids=` to the server (it falls through to no-op), so use a
      // sentinel id that won't match any real issue. The list endpoint
      // returns an empty page.
      out.ids = SENTINEL_NO_MATCH;
    } else {
      out.ids = extraIds.join(",");
    }
  }
  return out;
}
