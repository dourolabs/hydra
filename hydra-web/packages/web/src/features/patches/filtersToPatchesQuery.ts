import type { PatchStatus } from "@hydra/api";
import type { Filter } from "../filters";
import type { PatchFilters } from "../dashboard/usePaginatedPatches";

/**
 * Translate FilterBar state + free-text `q` into the `PatchFilters` query
 * shape consumed by `usePaginatedPatches` / `usePatchCount`. This is the only
 * mapping from FilterBar → server query: the Patches page does not narrow
 * client-side.
 *
 * Filter shapes:
 *   - `status` is multi-select; each value passes through as a `PatchStatus`
 *     entry on the server `?status[]=` array param.
 *   - `repository` is single-select; the sole value becomes `?repo_name=`.
 *   - `author` is single-select; the server `?creator=` expects a bare
 *     username, so the `users/` / `agents/` Principal-path prefix that
 *     `useUserOptions` surfaces is stripped here.
 *   - Relation filters (`relatedIssue` / `relatedSession`) do NOT map to a
 *     direct `listPatches` param. The caller resolves them to a set of patch
 *     ids via `/v1/relations` and passes the resolved set as `extraIds`.
 *   - `op: "not_in"` is not server-applicable for any filter today; such
 *     filters are silently dropped by this mapper. (`notInSupported` defaults
 *     to false in the FilterDefinition so the UI never produces them.)
 */
export interface BuildPatchesQueryArgs {
  filters: Filter[];
  q: string;
  /**
   * Patch ids returned by the relation resolver (or `null` when no relation
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

export function filtersToPatchesQuery({
  filters,
  q,
  extraIds,
}: BuildPatchesQueryArgs): PatchFilters {
  const out: PatchFilters = {};
  for (const filter of filters) {
    if (filter.op !== "in") continue;
    if (filter.values.length === 0) continue;
    switch (filter.id) {
      case "status":
        out.status = filter.values as PatchStatus[];
        break;
      case "repository":
        out.repo_name = filter.values[0];
        break;
      case "author":
        out.creator = stripUserPrefix(filter.values[0]);
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
      // The relation filter is active but matched no patches. We can't pass
      // an empty `ids=` to the server (it falls through to no-op), so use a
      // sentinel id that won't match any real patch. The list endpoint
      // returns an empty page.
      out.ids = SENTINEL_NO_MATCH;
    } else {
      out.ids = extraIds.join(",");
    }
  }
  return out;
}
