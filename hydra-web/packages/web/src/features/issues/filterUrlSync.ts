import type { ProjectRecord } from "@hydra/api";
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

/**
 * Transient URL param: the human-friendly project slug. Resolved to a
 * canonical `j-`-prefixed id at the page level and replaced with `?project=`
 * on the next URL write — see `resolveProjectFromUrl` below. Listed here so
 * `filtersToUrl` strips it when it rewrites the URL.
 *
 * Kept separate from `?project=` (which accepts only `j-`-prefixed ids) so
 * each parameter has a single, unambiguous value space; see
 * `docs/architecture/api-wire-contract.md` ("Parameter forms must be mutually
 * exclusive by construction").
 */
export const PROJECT_KEY_URL_PARAM = "project_key";

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
  // `project_key` is a transient resolution input; once the URL is rewritten
  // from filter state, the canonical `?project=j-<id>` carries the meaning.
  next.delete(PROJECT_KEY_URL_PARAM);
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

export type ProjectUrlResolution =
  | { outcome: "unchanged"; filters: Filter[] }
  | { outcome: "pending"; filters: Filter[] }
  | { outcome: "resolved"; filters: Filter[] }
  | { outcome: "missing"; filters: Filter[]; missingKey: string }
  | { outcome: "invalid"; filters: Filter[]; invalidValue: string };

function dropProjectFilter(filters: Filter[]): Filter[] {
  return filters.filter((f) => f.id !== "project");
}

function setProjectFilter(filters: Filter[], projectId: string): Filter[] {
  const others = filters.filter((f) => f.id !== "project");
  return [
    ...others,
    { _uid: "url:project", id: "project", op: "in", values: [projectId] },
  ];
}

/**
 * Validate `?project=` and resolve `?project_key=` for the Issues-list URL.
 *
 * Two URL params share the project-selection job, with disjoint value spaces:
 *
 *   - `?project=<j-id>` — the canonical form. Accepts ONLY `j-`-prefixed
 *     project ids; anything else is rejected with `outcome: "invalid"`.
 *   - `?project_key=<slug>` — the human-friendly form. Accepts ONLY non-`j-`
 *     slugs; a `j-`-prefixed value here is `outcome: "invalid"`. When the
 *     slug matches a known project, the returned filters carry the resolved
 *     `j-<id>` and the page rewrites the URL to the canonical `?project=`
 *     form on the next render.
 *
 * Splitting the parameter (instead of letting one accept both forms with
 * string-prefix disambiguation) is required by the wire-contract rule —
 * see `docs/architecture/api-wire-contract.md` ("Parameter forms must be
 * mutually exclusive by construction"). Each parameter has a single value
 * space, so a future project key shaped like `j-…` can't silently flip the
 * URL's meaning.
 *
 * Precedence when both URL params are present: `?project_key=` wins, because
 * the steady-state URL never contains both (the page rewrites `?project_key=`
 * away once resolved).
 *
 * Outcomes:
 *   - `unchanged` — no project params, or `?project=j-<id>` only.
 *   - `pending` — `?project_key=<slug>` is set but the projects list has
 *                 not loaded yet. Caller should hold off any server query
 *                 that would otherwise emit a stale project filter.
 *   - `resolved` — `?project_key=<slug>` matched a known project; the
 *                  returned filters carry the canonical `j-<id>`.
 *   - `missing` — `?project_key=<slug>` did not match any project; project
 *                  filter is dropped, `missingKey` carries the bad slug for
 *                  a toast.
 *   - `invalid` — `?project=<value>` was not `j-`-prefixed, OR
 *                  `?project_key=<value>` was `j-`-prefixed. Project filter
 *                  is dropped, `invalidValue` carries the bad token for a
 *                  toast. The page-level URL rewrite clears the bad param.
 */
export function resolveProjectFromUrl(
  filters: Filter[],
  searchParams: URLSearchParams,
  projects: ProjectRecord[] | undefined,
): ProjectUrlResolution {
  const rawKey = searchParams.get(PROJECT_KEY_URL_PARAM);
  if (rawKey) {
    // `?project_key=` owns the project selection when present.
    if (rawKey.startsWith("j-")) {
      // Value-space violation: ids belong in `?project=`. Drop the project
      // filter entirely — we deliberately don't try to fall back to a
      // co-present `?project=` because the user's URL is malformed and we
      // want the toast to be the only signal.
      return {
        outcome: "invalid",
        filters: dropProjectFilter(filters),
        invalidValue: rawKey,
      };
    }
    if (!projects) {
      return { outcome: "pending", filters };
    }
    const match = projects.find((p) => p.project.key === rawKey);
    if (match) {
      return {
        outcome: "resolved",
        filters: setProjectFilter(filters, match.project_id),
      };
    }
    return {
      outcome: "missing",
      filters: dropProjectFilter(filters),
      missingKey: rawKey,
    };
  }
  // No `?project_key=`. Validate the `?project=` value space.
  const projectFilter = filters.find((f) => f.id === "project");
  if (!projectFilter || projectFilter.values.length === 0) {
    return { outcome: "unchanged", filters };
  }
  const raw = projectFilter.values[0];
  if (raw.startsWith("j-")) {
    return { outcome: "unchanged", filters };
  }
  return {
    outcome: "invalid",
    filters: dropProjectFilter(filters),
    invalidValue: raw,
  };
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
