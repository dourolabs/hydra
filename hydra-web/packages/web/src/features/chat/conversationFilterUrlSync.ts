import type { Filter } from "../filters";

/**
 * URL-serialisation contract for the Chats page FilterBar.
 *
 * Each filter is keyed by its definition id in the URL as a single query param
 * (`?status=active`, `?creator=users/alice`). Both Chats filters are
 * single-select today. `op` is always `in`; neither filter's backing server
 * param can express `not_in`, so the FilterDefinitions leave `notInSupported`
 * unset.
 *
 * Adding a new server-applicable filter? Extend `CONVERSATION_URL_PARAMS`
 * below and `filtersToConversationsQuery.ts` in lockstep.
 */
export interface ConversationFilterUrlSpec {
  id: string;
  singleSelect: boolean;
}

export const CONVERSATION_URL_PARAMS: ConversationFilterUrlSpec[] = [
  { id: "status", singleSelect: true },
  { id: "creator", singleSelect: true },
];

const CONVERSATION_URL_PARAM_KEYS = new Set(
  CONVERSATION_URL_PARAMS.map((spec) => spec.id),
);

export const SEARCH_URL_PARAM = "q";

/** Legacy `?scope=mine|all` query param the FilterBar replaces. */
export const LEGACY_SCOPE_PARAM = "scope";

function parseValues(raw: string, singleSelect: boolean): string[] {
  if (singleSelect) return [raw];
  return raw
    .split(",")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

/**
 * Derive `Filter[]` from the current URL params. Each filter's `_uid` is
 * derived from the filter id so React keys stay stable across re-renders.
 *
 * `creator` accepts either a bare username (legacy URL shape, e.g.
 * `?creator=alice`) or a Principal path (`users/alice`). The user-options
 * list keys on Principal paths, so we normalise bare usernames to
 * `users/<name>` here.
 */
export function filtersFromUrl(params: URLSearchParams): Filter[] {
  const out: Filter[] = [];
  for (const spec of CONVERSATION_URL_PARAMS) {
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
 * params untouched. Also strips the legacy `?scope=` param: once the
 * FilterBar takes over filter state, leaving it in place would create a
 * stale shadow filter.
 */
export function filtersToUrl(
  prev: URLSearchParams,
  filters: Filter[],
): URLSearchParams {
  const next = new URLSearchParams(prev);
  for (const spec of CONVERSATION_URL_PARAMS) {
    next.delete(spec.id);
  }
  next.delete(LEGACY_SCOPE_PARAM);
  for (const filter of filters) {
    if (!CONVERSATION_URL_PARAM_KEYS.has(filter.id)) continue;
    if (filter.values.length === 0) continue;
    next.set(filter.id, filter.values.join(","));
  }
  return next;
}

/** Update the `?q=…` free-text param. Empty string clears the param. */
export function searchToUrl(prev: URLSearchParams, q: string): URLSearchParams {
  const next = new URLSearchParams(prev);
  if (q) {
    next.set(SEARCH_URL_PARAM, q);
  } else {
    next.delete(SEARCH_URL_PARAM);
  }
  return next;
}

/**
 * Resolve the legacy `?scope=mine|all` query param into an explicit
 * `creator` filter on first paint:
 *   - `?scope=mine` → `[{ id: 'creator', values: ['users/<currentUser>'] }]`
 *     when a current user is known; otherwise no-op.
 *   - `?scope=all`  → no filter (All-equivalent).
 *
 * Only applied when no explicit FilterBar params are already in the URL —
 * an explicit `?creator=` or `?status=` always wins over the legacy shim.
 * Returns `null` when no rewrite is needed; callers pass the result to
 * `setSearchParams` and seed local state from it.
 */
export interface LegacyScopeRedirect {
  filters: Filter[];
  nextParams: URLSearchParams;
}

export function legacyScopeRedirect(
  params: URLSearchParams,
  currentUser: string | null,
): LegacyScopeRedirect | null {
  const scope = params.get(LEGACY_SCOPE_PARAM);
  if (!scope) return null;
  // If any explicit FilterBar param is already set, the user has navigated to
  // a fresh URL — drop the legacy param without overriding.
  for (const spec of CONVERSATION_URL_PARAMS) {
    if (params.has(spec.id)) {
      const cleaned = new URLSearchParams(params);
      cleaned.delete(LEGACY_SCOPE_PARAM);
      return { filters: filtersFromUrl(cleaned), nextParams: cleaned };
    }
  }
  const next = new URLSearchParams(params);
  next.delete(LEGACY_SCOPE_PARAM);
  const filters: Filter[] = [];
  if (scope === "mine" && currentUser) {
    filters.push({
      _uid: "url:creator",
      id: "creator",
      op: "in",
      values: [`users/${currentUser}`],
    });
    next.set("creator", `users/${currentUser}`);
  }
  // `scope=all` resolves to "no filter" — leave `filters` empty.
  return { filters, nextParams: next };
}

/**
 * Seed the default `creator` filter on first visit (no FilterBar state in
 * URL, no legacy `?scope=`). Mirrors the previous Mine-by-default behaviour:
 * the chip is auto-added so users can see + remove it like any other filter.
 */
export function defaultCreatorFilter(currentUser: string | null): Filter[] {
  if (!currentUser) return [];
  return [
    {
      _uid: "url:creator",
      id: "creator",
      op: "in",
      values: [`users/${currentUser}`],
    },
  ];
}
