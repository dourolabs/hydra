import type { Filter } from "../filters";

/**
 * URL-serialisation contract for the Sessions page FilterBar. Mirrors
 * `filterUrlSync.ts` on the Issues page: each filter is keyed by definition
 * id and held as a single query param (`?status=running,pending`,
 * `?relatedPatch=p-aa,p-bb`). Single-select filters take a bare value;
 * multi-select filters take a comma-separated list.
 */
export interface SessionFilterUrlSpec {
  id: string;
  singleSelect: boolean;
}

export const SESSION_URL_PARAMS: SessionFilterUrlSpec[] = [
  { id: "status", singleSelect: false },
  { id: "creator", singleSelect: true },
  { id: "relatedIssue", singleSelect: false },
  { id: "relatedChat", singleSelect: true },
  { id: "relatedPatch", singleSelect: false },
];

const SESSION_URL_PARAM_KEYS = new Set(
  SESSION_URL_PARAMS.map((spec) => spec.id),
);

export const SESSION_SEARCH_URL_PARAM = "q";

export const SESSION_LEGACY_SCOPE_PARAM = "scope";

function parseValues(raw: string, singleSelect: boolean): string[] {
  if (singleSelect) return [raw];
  return raw
    .split(",")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

function normaliseCreatorValue(value: string): string {
  if (value.startsWith("users/") || value.startsWith("agents/")) return value;
  return `users/${value}`;
}

/**
 * Derive `Filter[]` from the current URL params. `_uid` is the filter id —
 * URL persistence implies a single instance per definition, so a stable
 * id-as-uid keeps React keys consistent across re-renders.
 *
 * `creator` accepts either a bare username (the legacy URL shape, e.g.
 * `?creator=alice`) or a Principal path (`users/alice`); we normalise bare
 * usernames to `users/<name>` so the FilterBar's value picker can match
 * against `useUserOptions`.
 */
export function sessionFiltersFromUrl(params: URLSearchParams): Filter[] {
  const out: Filter[] = [];
  for (const spec of SESSION_URL_PARAMS) {
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

/**
 * Write the FilterBar state back to the URL params. Also strips the legacy
 * `?scope=` param — once the FilterBar takes over creator-state, leaving
 * `?scope=` behind would conflict with explicit creator-chip mutations.
 */
export function sessionFiltersToUrl(
  prev: URLSearchParams,
  filters: Filter[],
): URLSearchParams {
  const next = new URLSearchParams(prev);
  for (const spec of SESSION_URL_PARAMS) {
    next.delete(spec.id);
  }
  next.delete(SESSION_LEGACY_SCOPE_PARAM);
  for (const filter of filters) {
    if (!SESSION_URL_PARAM_KEYS.has(filter.id)) continue;
    if (filter.values.length === 0) continue;
    next.set(filter.id, filter.values.join(","));
  }
  return next;
}

/** Update the `?q=…` free-text param; empty string clears it. */
export function sessionSearchToUrl(
  prev: URLSearchParams,
  q: string,
): URLSearchParams {
  const next = new URLSearchParams(prev);
  if (q) {
    next.set(SESSION_SEARCH_URL_PARAM, q);
  } else {
    next.delete(SESSION_SEARCH_URL_PARAM);
  }
  return next;
}

/**
 * Translate the legacy `?scope=mine` / `?scope=all` URL shortcut into the
 * equivalent `creator` chip (using the current user's Principal path) on
 * first paint. Returns the original filter list when no legacy scope is
 * present, when explicit filter params are already in the URL, or when the
 * current user is not known.
 *
 * `?scope=all` strips the auto-seeded creator chip (explicit "All" view).
 *
 * Callers strip `?scope=` from the URL alongside writing the seed; the
 * existing `sessionFiltersToUrl` helper deletes the param whenever the user
 * mutates filters, but the page also has to clear it on initial paint so the
 * URL stays canonical.
 */
export function applyLegacyScope(
  filters: Filter[],
  scope: string | null,
  currentUserPrincipalPath: string | null,
  hasExplicitFilterParam: boolean,
): Filter[] {
  if (hasExplicitFilterParam) return filters;
  if (!scope) return filters;
  if (scope === "all") return filters;
  if (scope === "mine" && currentUserPrincipalPath) {
    return [
      {
        _uid: "url:creator",
        id: "creator",
        op: "in",
        values: [currentUserPrincipalPath],
      },
    ];
  }
  return filters;
}

/**
 * True if at least one explicit FilterBar param is present on the URL. Used
 * to decide whether the auto-seeded `creator=<me>` (Mine-as-default) chip
 * should be added on first paint.
 */
export function hasAnySessionFilterParam(params: URLSearchParams): boolean {
  for (const spec of SESSION_URL_PARAMS) {
    if (params.has(spec.id)) return true;
  }
  return false;
}
