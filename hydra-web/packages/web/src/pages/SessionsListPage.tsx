import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";
import type { SessionSummaryRecord } from "@hydra/api";
import { SessionsView } from "../features/sessions/view/SessionsView";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import { useAuth } from "../features/auth/useAuth";
import { actorPrincipalPath } from "../api/auth";
import {
  usePaginatedSessions,
  useSessionCount,
  type SessionFilters,
} from "../features/sessions/usePaginatedSessions";
import { sortSessions } from "../features/sessions/sortSessions";
import { useSessionFilters } from "../features/sessions/sessionFilters";
import {
  sessionFiltersFromUrl,
  sessionFiltersToUrl,
  sessionSearchToUrl,
  applyLegacyScope,
  hasAnySessionFilterParam,
  SESSION_SEARCH_URL_PARAM,
  SESSION_LEGACY_SCOPE_PARAM,
} from "../features/sessions/sessionFilterUrlSync";
import { filtersToSessionsQuery } from "../features/sessions/filtersToSessionsQuery";
import { useRelationFilteredSessionIds } from "../features/sessions/useRelationFilteredSessionIds";
import type { Filter } from "../features/filters";

// Canonical, uid-free string repr used to detect URL ↔ local-state drift.
// Empty-value filters are deliberately invisible (FilterBar adds an empty
// chip before the user picks a value; we don't want that intermediate state
// to round-trip through the URL).
function filtersCanonicalRepr(filters: Filter[]): string {
  return filters
    .filter((f) => f.values.length > 0)
    .map((f) => `${f.id}:${f.op}:${[...f.values].sort().join(",")}`)
    .sort()
    .join("|");
}

function seedFilters(
  params: URLSearchParams,
  currentUserPrincipalPath: string | null,
): Filter[] {
  const fromUrl = sessionFiltersFromUrl(params);
  if (fromUrl.length > 0) return fromUrl;
  const scope = params.get(SESSION_LEGACY_SCOPE_PARAM);
  const hasExplicit = hasAnySessionFilterParam(params);
  if (scope !== null) {
    return applyLegacyScope(
      fromUrl,
      scope,
      currentUserPrincipalPath,
      hasExplicit,
    );
  }
  // First visit (no filter params, no legacy `?scope=`): auto-seed creator
  // chip with the current user. Mirrors the previous Mine-as-default UX.
  if (currentUserPrincipalPath) {
    return [
      {
        _uid: "url:creator",
        id: "creator",
        op: "in",
        values: [currentUserPrincipalPath],
      },
    ];
  }
  return [];
}

export function SessionsListPage() {
  const [searchParams, setSearchParams] = useSearchParams();
  const { user } = useAuth();
  const currentUserPrincipalPath = user ? actorPrincipalPath(user.actor) : null;

  // Filters mirrored between URL and local state. Local state is the source
  // of truth for chip `_uid`s (anchors the FilterBar's just-added value
  // picker); URL is the source of truth for shareable/back-buttonable state.
  // `seededOnceRef` makes sure the Mine-as-default auto-seed only fires on
  // the literal first paint — if the user clears all filters, we don't want
  // them to re-appear on the next render.
  const seededOnceRef = useRef(false);
  const [filters, setFiltersState] = useState<Filter[]>(() => {
    seededOnceRef.current = true;
    return seedFilters(searchParams, currentUserPrincipalPath);
  });

  // Lazy-load gate for the relation-picker option lists in `useSessionFilters`:
  // flipped on while the FilterBar's add-filter menu is open so the picker
  // isn't empty when the user clicks "Related X". Combined inside the hook
  // with a check on `filters` so URL-rehydrated relation chips also enable
  // the right list immediately on first paint.
  const [addMenuOpen, setAddMenuOpen] = useState(false);

  const definitions = useSessionFilters({ filters, addMenuOpen });

  // On the very first paint, persist the seed (auto-creator chip and/or
  // legacy `?scope=mine` translation) back to the URL so deep links share
  // the canonical shape. `replace: true` keeps the back stack clean.
  const persistedSeedRef = useRef(false);
  useEffect(() => {
    if (persistedSeedRef.current) return;
    persistedSeedRef.current = true;
    const hasFilterParam = hasAnySessionFilterParam(searchParams);
    const hasLegacyScope =
      searchParams.get(SESSION_LEGACY_SCOPE_PARAM) !== null;
    if (!hasFilterParam && !hasLegacyScope && filters.length === 0) {
      return;
    }
    setSearchParams((prev) => sessionFiltersToUrl(prev, filters), {
      replace: true,
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const fromUrl = sessionFiltersFromUrl(searchParams);
    // External URL nav: don't re-apply the auto-seed (we only want it on the
    // literal initial mount). Treat the legacy scope param the same way
    // applyLegacyScope does in `seedFilters`.
    const scope = searchParams.get(SESSION_LEGACY_SCOPE_PARAM);
    const hasExplicit = hasAnySessionFilterParam(searchParams);
    let target = fromUrl;
    if (scope !== null && !hasExplicit) {
      target = applyLegacyScope(
        fromUrl,
        scope,
        currentUserPrincipalPath,
        false,
      );
    }
    if (filtersCanonicalRepr(filters) !== filtersCanonicalRepr(target)) {
      setFiltersState(target);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchParams, currentUserPrincipalPath]);

  // Debounced free-text search: `searchValue` is what the user typed,
  // `searchQuery` is what we send to the server (and persist to URL) after a
  // 300ms quiet period.
  const [searchValue, setSearchValue] = useState(
    searchParams.get(SESSION_SEARCH_URL_PARAM) ?? "",
  );
  const [searchQuery, setSearchQuery] = useState(
    searchParams.get(SESSION_SEARCH_URL_PARAM) ?? "",
  );
  const debounceRef = useRef<ReturnType<typeof setTimeout>>(undefined);

  const handleSearchChange = useCallback(
    (value: string) => {
      setSearchValue(value);
      clearTimeout(debounceRef.current);
      debounceRef.current = setTimeout(() => {
        setSearchQuery(value);
        setSearchParams((prev) => sessionSearchToUrl(prev, value), {
          replace: true,
        });
      }, 300);
    },
    [setSearchParams],
  );

  useEffect(() => () => clearTimeout(debounceRef.current), []);

  // External URL changes win over the local search state.
  useEffect(() => {
    const urlQ = searchParams.get(SESSION_SEARCH_URL_PARAM) ?? "";
    if (urlQ !== searchQuery) {
      setSearchValue(urlQ);
      setSearchQuery(urlQ);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchParams]);

  const setFilters = useCallback(
    (next: Filter[]) => {
      setFiltersState(next);
      setSearchParams((prev) => sessionFiltersToUrl(prev, next), {
        replace: false,
      });
    },
    [setSearchParams],
  );

  useBreadcrumbs([{ label: "Workspace", to: "/" }], "Sessions");

  // Resolve `relatedPatch` into a concrete issue id set (`spawned_from_ids`).
  // Hold `listSessions` until the resolver lands so the first paint reflects
  // the URL state exactly.
  const { patchIssueIds, isLoading: relationsLoading } =
    useRelationFilteredSessionIds(filters);

  const serverFilters = useMemo<SessionFilters>(
    () =>
      filtersToSessionsQuery({
        filters,
        q: searchQuery,
        patchIssueIds,
      }),
    [filters, searchQuery, patchIssueIds],
  );

  const queriesEnabled = !relationsLoading;

  const {
    data: paginatedData,
    isLoading,
    error,
    fetchNextPage,
    hasNextPage,
    isFetchingNextPage,
  } = usePaginatedSessions(serverFilters, queriesEnabled);

  const { data: totalCount } = useSessionCount(serverFilters, queriesEnabled);

  const rows = useMemo<SessionSummaryRecord[]>(() => {
    const flat = paginatedData?.pages.flatMap((p) => p.sessions) ?? [];
    const seen = new Set<string>();
    const deduped: SessionSummaryRecord[] = [];
    for (const rec of flat) {
      if (seen.has(rec.session_id)) continue;
      seen.add(rec.session_id);
      deduped.push(rec);
    }
    return sortSessions(deduped);
  }, [paginatedData]);

  const handleLoadMore = useCallback(() => {
    if (hasNextPage && !isFetchingNextPage) fetchNextPage();
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);

  const displayCount = totalCount ?? rows.length;
  const eyebrow = `WORK · ${displayCount === 1 ? "1 SESSION" : `${displayCount} SESSIONS`}`;

  return (
    <SessionsView
      rows={rows}
      isLoading={isLoading || relationsLoading}
      error={error ?? null}
      hasNextPage={hasNextPage ?? false}
      isFetchingNextPage={isFetchingNextPage ?? false}
      onLoadMore={handleLoadMore}
      eyebrow={eyebrow}
      filters={filters}
      setFilters={setFilters}
      definitions={definitions}
      filteredCount={rows.length}
      totalCount={displayCount}
      searchValue={searchValue}
      onSearchChange={handleSearchChange}
      onFilterMenuOpenChange={setAddMenuOpen}
    />
  );
}
