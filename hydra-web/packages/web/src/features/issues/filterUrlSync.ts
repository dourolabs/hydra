import type { Filter } from "../filters";

/**
 * URL-serialisation contract for the Issues page FilterBar.
 *
 * Each Issues-page filter is keyed by its definition id in the URL as a single
 * query param (`?status=open`, `?relatedPatch=p-aa,p-bb`). Single-select
 * filters take a bare value; multi-select filters take a comma-separated
 * list. The `op` is always `in` for PR-1 — none of the entries below back a
 * server param that can express `not_in`, so the FilterDefinition for each
 * leaves `notInSupported` unset.
 *
 * Adding a new server-applicable filter? Extend `FILTER_URL_PARAMS` below and
 * `filtersToIssuesQuery.ts` in lockstep.
 */
export interface IssueFilterUrlSpec {
  /** FilterDefinition id (matches the key in ISSUE_FILTERS). */
  id: string;
  /** When true, the URL holds a single value; otherwise a comma-separated list. */
  singleSelect: boolean;
}

export const FILTER_URL_PARAMS: IssueFilterUrlSpec[] = [
  { id: "status", singleSelect: true },
  { id: "type", singleSelect: true },
  { id: "creator", singleSelect: true },
  { id: "assignee", singleSelect: true },
  { id: "project", singleSelect: true },
  { id: "relatedPatch", singleSelect: false },
  { id: "relatedChat", singleSelect: false },
  { id: "relatedSession", singleSelect: false },
  { id: "parentOrChild", singleSelect: false },
];

const FILTER_URL_PARAM_KEYS = new Set(FILTER_URL_PARAMS.map((spec) => spec.id));

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
 * `creator` accepts either a bare username (the legacy URL shape, e.g.
 * `?creator=alice`) or a Principal path (`users/alice`). The user-options
 * list keys on Principal paths, so we normalise bare usernames to
 * `users/<name>` here. Assignee already arrives Principal-shaped on the wire.
 */
export function filtersFromUrl(params: URLSearchParams): Filter[] {
  const out: Filter[] = [];
  for (const spec of FILTER_URL_PARAMS) {
    const raw = params.get(spec.id);
    if (!raw) continue;
    let values = parseValues(raw, spec.singleSelect);
    if (values.length === 0) continue;
    if (spec.id === "creator") {
      values = values.map((v) => normaliseCreatorValue(v));
    }
    out.push({ _uid: `url:${spec.id}`, id: spec.id, op: "in", values });
  }
  return out;
}

function normaliseCreatorValue(value: string): string {
  if (value.startsWith("users/") || value.startsWith("agents/")) return value;
  return `users/${value}`;
}

/**
 * Write the FilterBar state back to the URL params, leaving any non-filter
 * params (eg `selected`) untouched. Returns a fresh `URLSearchParams` that the
 * caller can pass to `setSearchParams`.
 */
export function filtersToUrl(
  prev: URLSearchParams,
  filters: Filter[],
): URLSearchParams {
  const next = new URLSearchParams(prev);
  for (const spec of FILTER_URL_PARAMS) {
    next.delete(spec.id);
  }
  // Once the FilterBar takes over filter state, the legacy `selected=`
  // shortcut would conflict with the explicit params we're writing.
  next.delete("selected");
  for (const filter of filters) {
    if (!FILTER_URL_PARAM_KEYS.has(filter.id)) continue;
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
    next.delete("selected");
  } else {
    next.delete(SEARCH_URL_PARAM);
  }
  return next;
}
