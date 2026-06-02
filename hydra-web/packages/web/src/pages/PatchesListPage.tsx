import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";
import type { PatchSummaryRecord } from "@hydra/api";
import {
  usePaginatedPatches,
  usePatchCount,
  type PatchFilters,
} from "../features/dashboard/usePaginatedPatches";
import { PatchesView } from "../features/patches/view/PatchesView";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import { usePatchFilters } from "../features/patches/patchFilters";
import {
  filtersFromUrl,
  filtersToUrl,
  searchToUrl,
  SEARCH_URL_PARAM,
} from "../features/patches/patchFilterUrlSync";
import { filtersToPatchesQuery } from "../features/patches/filtersToPatchesQuery";
import {
  useRelationFilteredPatchIds,
  RELATION_FILTER_IDS,
} from "../features/patches/useRelationFilteredPatchIds";
import type { Filter } from "../features/filters";

function formatEyebrow(count: number): string {
  const n = count === 1 ? "1 PATCH" : `${count} PATCHES`;
  return `WORK · ${n}`;
}

// Canonical, uid-free string repr used to detect whether the URL state and
// the local FilterBar state are in sync. Empty-values filters represent an
// in-flight FilterBar add (user picked a definition from the menu but hasn't
// chosen values yet) and are deliberately invisible to the URL — including
// them here would force a sync cycle that drops the just-added chip before
// the user can pick a value.
function filtersCanonicalRepr(filters: Filter[]): string {
  return filters
    .filter((f) => f.values.length > 0)
    .map((f) => `${f.id}:${f.op}:${[...f.values].sort().join(",")}`)
    .sort()
    .join("|");
}

export function PatchesListPage() {
  useBreadcrumbs([{ label: "Workspace", to: "/" }], "Patches");

  const [searchParams, setSearchParams] = useSearchParams();
  const definitions = usePatchFilters();

  // Filters are mirrored between URL params and local state. The local state
  // is the source of truth for chip `_uid`s (used by FilterBar to anchor the
  // "just-added" value picker), and the URL is the source of truth for
  // shareable/back-buttonable state.
  const [filters, setFiltersState] = useState<Filter[]>(() =>
    filtersFromUrl(searchParams),
  );

  useEffect(() => {
    const fromUrl = filtersFromUrl(searchParams);
    if (filtersCanonicalRepr(filters) !== filtersCanonicalRepr(fromUrl)) {
      setFiltersState(fromUrl);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchParams]);

  // Debounced free-text search: `searchValue` is the user-typed string,
  // `searchQuery` is what we actually send to the server / write to the URL
  // after a 300ms quiet period.
  const [searchValue, setSearchValue] = useState(
    searchParams.get(SEARCH_URL_PARAM) ?? "",
  );
  const [searchQuery, setSearchQuery] = useState(
    searchParams.get(SEARCH_URL_PARAM) ?? "",
  );
  const debounceRef = useRef<ReturnType<typeof setTimeout>>(undefined);

  const handleSearchChange = useCallback(
    (value: string) => {
      setSearchValue(value);
      clearTimeout(debounceRef.current);
      debounceRef.current = setTimeout(() => {
        setSearchQuery(value);
        setSearchParams((prev) => searchToUrl(prev, value), { replace: true });
      }, 300);
    },
    [setSearchParams],
  );

  useEffect(() => () => clearTimeout(debounceRef.current), []);

  // External URL changes (back/forward, sidebar nav) win over local state.
  useEffect(() => {
    const urlQ = searchParams.get(SEARCH_URL_PARAM) ?? "";
    if (urlQ !== searchQuery) {
      setSearchValue(urlQ);
      setSearchQuery(urlQ);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchParams]);

  const setFilters = useCallback(
    (next: Filter[]) => {
      setFiltersState(next);
      setSearchParams((prev) => filtersToUrl(prev, next), { replace: false });
    },
    [setSearchParams],
  );

  // Resolve relation filters into a concrete patch id set the server can
  // narrow on via `ids=`. Holds off `listPatches` while the resolver is in
  // flight so the initial paint matches the URL state.
  const { patchIds: relationIds, isLoading: relationsLoading } =
    useRelationFilteredPatchIds(filters);

  const hasActiveRelationFilter = filters.some(
    (f) => RELATION_FILTER_IDS.includes(f.id) && f.values.length > 0,
  );

  const serverFilters = useMemo<PatchFilters>(
    () =>
      filtersToPatchesQuery({
        filters,
        q: searchQuery,
        extraIds: hasActiveRelationFilter ? relationIds ?? [] : null,
      }),
    [filters, searchQuery, hasActiveRelationFilter, relationIds],
  );

  const enabled = !relationsLoading;

  const {
    data,
    isLoading,
    fetchNextPage,
    hasNextPage,
    isFetchingNextPage,
  } = usePaginatedPatches(serverFilters, enabled);

  const { data: totalCount } = usePatchCount(serverFilters, enabled);

  const patches = useMemo<PatchSummaryRecord[]>(() => {
    const seen = new Set<string>();
    return (data?.pages.flatMap((p) => p.patches) ?? []).filter((rec) => {
      if (seen.has(rec.patch_id)) return false;
      seen.add(rec.patch_id);
      return true;
    });
  }, [data]);

  const displayCount = totalCount ?? patches.length;

  const handleLoadMore = useCallback(() => {
    if (hasNextPage && !isFetchingNextPage) fetchNextPage();
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);

  return (
    <PatchesView
      patches={patches}
      isLoading={isLoading || relationsLoading}
      hasNextPage={hasNextPage ?? false}
      isFetchingNextPage={isFetchingNextPage ?? false}
      onLoadMore={handleLoadMore}
      eyebrow={formatEyebrow(displayCount)}
      filters={filters}
      setFilters={setFilters}
      definitions={definitions}
      filteredCount={patches.length}
      totalCount={displayCount}
      searchValue={searchValue}
      onSearchChange={handleSearchChange}
    />
  );
}
