import type { Filter } from "../filters";

/**
 * URL-serialisation contract for the Patches page FilterBar.
 *
 * Each Patches-page filter is keyed by its definition id in the URL as a
 * single query param (`?status=Open,Merged`, `?relatedIssue=i-aa,i-bb`).
 * Single-select filters take a bare value; multi-select filters take a
 * comma-separated list. The `op` is always `in` for PR-2 — none of the entries
 * below back a server param that can express `not_in`.
 *
 * Adding a new server-applicable filter? Extend `PATCH_URL_PARAMS` below and
 * `filtersToPatchesQuery.ts` in lockstep.
 */
export interface PatchFilterUrlSpec {
  /** FilterDefinition id (matches the key in PATCH_FILTERS). */
  id: string;
  /** When true, the URL holds a single value; otherwise a comma-separated list. */
  singleSelect: boolean;
}

export const PATCH_URL_PARAMS: PatchFilterUrlSpec[] = [
  { id: "status", singleSelect: false },
  { id: "repository", singleSelect: true },
  { id: "author", singleSelect: true },
  { id: "relatedIssue", singleSelect: false },
  { id: "relatedSession", singleSelect: false },
];

const PATCH_URL_PARAM_KEYS = new Set(PATCH_URL_PARAMS.map((spec) => spec.id));

export const SEARCH_URL_PARAM = "q";

function parseValues(raw: string, singleSelect: boolean): string[] {
  if (singleSelect) return [raw];
  return raw
    .split(",")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

/**
 * Derive `Filter[]` from the current URL params. Each filter's `_uid` is the
 * filter id — URL persistence implies one instance per definition, so a
 * stable id-as-uid keeps React keys consistent across re-renders.
 *
 * `author` accepts either a bare username (e.g. `?author=alice`) or a
 * Principal path (`users/alice`). The user-options list keys on Principal
 * paths, so bare usernames are normalised to `users/<name>` here.
 */
export function filtersFromUrl(params: URLSearchParams): Filter[] {
  const out: Filter[] = [];
  for (const spec of PATCH_URL_PARAMS) {
    const raw = params.get(spec.id);
    if (!raw) continue;
    let values = parseValues(raw, spec.singleSelect);
    if (values.length === 0) continue;
    if (spec.id === "author") {
      values = values.map((v) => normaliseAuthorValue(v));
    }
    out.push({ _uid: `url:${spec.id}`, id: spec.id, op: "in", values });
  }
  return out;
}

function normaliseAuthorValue(value: string): string {
  if (value.startsWith("users/") || value.startsWith("agents/")) return value;
  return `users/${value}`;
}

/**
 * Write the FilterBar state back to the URL params, leaving any non-filter
 * params untouched. Returns a fresh `URLSearchParams` that the caller can
 * pass to `setSearchParams`.
 */
export function filtersToUrl(
  prev: URLSearchParams,
  filters: Filter[],
): URLSearchParams {
  const next = new URLSearchParams(prev);
  for (const spec of PATCH_URL_PARAMS) {
    next.delete(spec.id);
  }
  for (const filter of filters) {
    if (!PATCH_URL_PARAM_KEYS.has(filter.id)) continue;
    if (filter.values.length === 0) continue;
    next.set(filter.id, filter.values.join(","));
  }
  return next;
}

/**
 * Update the `?q=…` free-text param. Empty string clears the param.
 */
export function searchToUrl(prev: URLSearchParams, q: string): URLSearchParams {
  const next = new URLSearchParams(prev);
  if (q) {
    next.set(SEARCH_URL_PARAM, q);
  } else {
    next.delete(SEARCH_URL_PARAM);
  }
  return next;
}
