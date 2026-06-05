import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";
import {
  usePaginatedIssues,
  useIssueCount,
  type IssueFilters,
} from "../features/issues/usePaginatedIssues";
import { useAuth } from "../features/auth/useAuth";
import { actorDisplayName, actorPrincipalPath } from "../api/auth";
import {
  IssuesView,
  type IssuesLayout,
} from "../features/issues/view/IssuesView";
import { usePageIssueTrees } from "../features/dashboard/usePageIssueTrees";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import { useIssueFilters } from "../features/issues/issueFilters";
import {
  filtersFromUrl,
  filtersToUrl,
  searchToUrl,
  SEARCH_URL_PARAM,
} from "../features/issues/filterUrlSync";
import { filtersToIssuesQuery } from "../features/issues/filtersToIssuesQuery";
import {
  useRelationFilteredIssueIds,
  RELATION_FILTER_IDS,
} from "../features/issues/useRelationFilteredIssueIds";
import type { Filter } from "../features/filters";
import styles from "./IssuesListPage.module.css";

// Map legacy `?selected=…` shortcut values onto the explicit filter URL params
// so e2e tests and bookmarked URLs continue to work after the sidebar switched
// to the new scheme.
const LEGACY_SELECTED_VALUES = new Set([
  "your-issues",
  "assigned",
  "all",
  "in_progress",
]);

const LAYOUT_STORAGE_KEY = "hydra:issues:layout";

function readLayout(): IssuesLayout {
  if (typeof window === "undefined") return "table";
  try {
    const v = window.localStorage.getItem(LAYOUT_STORAGE_KEY);
    if (v === "board" || v === "table") return v;
  } catch {
    /* ignore */
  }
  return "table";
}

function writeLayout(layout: IssuesLayout): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(LAYOUT_STORAGE_KEY, layout);
  } catch {
    /* ignore */
  }
}

// Translate `?selected=<shortcut>` into the equivalent explicit filter chips
// so the FilterBar (and the URL-derived eyebrow / title) stay in sync. The
// `selected` param itself is left in place by this helper — the writer strips
// it whenever the user mutates filters explicitly (see `filtersToUrl`).
function applyLegacySelected(
  filters: Filter[],
  selected: string | null,
  currentUser: string,
  currentPrincipalPath: string | null,
): Filter[] {
  if (!selected || !LEGACY_SELECTED_VALUES.has(selected)) return filters;
  const hasExplicit = filters.length > 0;
  if (hasExplicit) return filters;
  if (selected === "your-issues" && currentUser) {
    return [
      {
        _uid: "url:creator",
        id: "creator",
        op: "in",
        values: [`users/${currentUser}`],
      },
    ];
  }
  if (selected === "assigned" && currentPrincipalPath) {
    return [
      {
        _uid: "url:assignee",
        id: "assignee",
        op: "in",
        values: [currentPrincipalPath],
      },
    ];
  }
  if (selected === "in_progress") {
    return [
      {
        _uid: "url:status",
        id: "status",
        op: "in",
        values: ["in-progress"],
      },
    ];
  }
  // `selected=all` — no implicit filter.
  return filters;
}

interface FramingState {
  rootId: string;
  title: string;
  eyebrowPrefix: string;
}

function describeFraming(
  filters: Filter[],
  searchValue: string,
  currentUser: string,
  currentPrincipalPath: string | null,
): FramingState {
  if (searchValue.trim()) {
    return { rootId: "filtered", title: "Issues", eyebrowPrefix: "FILTERED" };
  }

  const onlyCreator =
    filters.length === 1 &&
    filters[0].id === "creator" &&
    filters[0].values.length === 1 &&
    !!currentUser &&
    filters[0].values[0] === `users/${currentUser}`;
  if (onlyCreator) {
    return { rootId: "your-issues", title: "My issues", eyebrowPrefix: "MINE" };
  }

  const onlyAssignee =
    filters.length === 1 &&
    filters[0].id === "assignee" &&
    filters[0].values.length === 1 &&
    !!currentPrincipalPath &&
    filters[0].values[0] === currentPrincipalPath;
  if (onlyAssignee) {
    return {
      rootId: "assigned",
      title: "Assigned to me",
      eyebrowPrefix: "ASSIGNED",
    };
  }

  const onlyInProgress =
    filters.length === 1 &&
    filters[0].id === "status" &&
    filters[0].values.length === 1 &&
    filters[0].values[0] === "in-progress";
  if (onlyInProgress) {
    return {
      rootId: "in_progress",
      title: "In progress",
      eyebrowPrefix: "IN PROGRESS",
    };
  }

  if (filters.length === 0) {
    return { rootId: "all", title: "All issues", eyebrowPrefix: "ALL" };
  }

  return { rootId: "filtered", title: "Issues", eyebrowPrefix: "FILTERED" };
}

function formatEyebrow(prefix: string, count: number): string {
  const n = count === 1 ? "1 ISSUE" : `${count} ISSUES`;
  return `${prefix} · ${n}`;
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

export function IssuesListPage() {
  const [searchParams, setSearchParams] = useSearchParams();
  const { user } = useAuth();
  const currentUser = user ? actorDisplayName(user.actor) : "";
  const currentPrincipalPath = user ? actorPrincipalPath(user.actor) : null;

  // Filters are mirrored between URL params and local state. The local state
  // is the source of truth for chip `_uid`s (used by FilterBar to anchor the
  // "just-added" value picker), and the URL is the source of truth for
  // shareable/back-buttonable state. When the URL changes externally
  // (sidebar nav, back/forward) we sync into state; when the user mutates
  // chips inside the bar, we write through to the URL ourselves.
  const [filters, setFiltersState] = useState<Filter[]>(() =>
    applyLegacySelected(
      filtersFromUrl(searchParams),
      searchParams.get("selected"),
      currentUser,
      currentPrincipalPath,
    ),
  );

  // Lazy-load gate for the relation-picker option lists in `useIssueFilters`:
  // flipped on while the FilterBar's add-filter menu is open so the picker
  // isn't empty when the user clicks "Related X". Combined inside the hook
  // with a check on `filters` so URL-rehydrated relation chips also enable
  // the right list immediately on first paint.
  const [addMenuOpen, setAddMenuOpen] = useState(false);

  const definitions = useIssueFilters({ filters, addMenuOpen });

  useEffect(() => {
    const fromUrl = applyLegacySelected(
      filtersFromUrl(searchParams),
      searchParams.get("selected"),
      currentUser,
      currentPrincipalPath,
    );
    if (filtersCanonicalRepr(filters) !== filtersCanonicalRepr(fromUrl)) {
      setFiltersState(fromUrl);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchParams, currentUser, currentPrincipalPath]);

  // Debounced free-text search: `searchValue` is the user-typed string,
  // `searchQuery` is what we actually send to the server / write to the URL
  // after a 300ms quiet period. Mirrors the previous behaviour pre-FilterBar.
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

  // External URL changes (sidebar nav, back button) win over local state.
  // Re-seed the search input from the URL when it diverges from our last
  // debounced value.
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

  const framing = describeFraming(
    filters,
    searchQuery,
    currentUser,
    currentPrincipalPath,
  );
  useBreadcrumbs([{ label: "Workspace", to: "/" }], framing.title);

  const [layout, setLayout] = useState<IssuesLayout>(readLayout);
  useEffect(() => {
    writeLayout(layout);
  }, [layout]);
  const isTable = layout === "table";

  // Resolve relation filters into a concrete issue id set the server can
  // narrow on via `ids=`. Holds off `listIssues` while the resolver is in
  // flight so the initial paint matches the URL state.
  const { issueIds: relationIds, isLoading: relationsLoading } =
    useRelationFilteredIssueIds(filters);

  // Distinguish "no relation filter is active" (null) from "active but
  // matched nothing" (empty array). `filtersToIssuesQuery` translates the
  // empty case into a sentinel `ids=` that returns zero rows.
  const hasActiveRelationFilter = filters.some(
    (f) => RELATION_FILTER_IDS.includes(f.id) && f.values.length > 0,
  );

  const tableServerFilters = useMemo<IssueFilters>(
    () =>
      filtersToIssuesQuery({
        filters,
        q: searchQuery,
        extraIds: hasActiveRelationFilter ? relationIds ?? [] : null,
      }),
    [filters, searchQuery, hasActiveRelationFilter, relationIds],
  );

  // Board mode still consumes the legacy URL-derived shape; keep the
  // historical mapping so chip navigation between table → board carries
  // forward (status / assignee / creator chips, modelled the old way).
  const boardBaseFilters = useMemo<IssueFilters>(() => {
    const f = filtersToIssuesQuery({
      filters,
      q: "",
      extraIds: null,
    });
    return f;
  }, [filters]);

  const tableEnabled = isTable && !relationsLoading;

  const {
    data: paginatedData,
    isLoading,
    fetchNextPage,
    hasNextPage,
    isFetchingNextPage,
  } = usePaginatedIssues(tableServerFilters, tableEnabled);

  const { data: totalCount } = useIssueCount(tableServerFilters, tableEnabled);

  const issues = useMemo(() => {
    const seen = new Set<string>();
    return (paginatedData?.pages.flatMap((page) => page.issues) ?? []).filter(
      (issue) => {
        if (seen.has(issue.issue_id)) return false;
        seen.add(issue.issue_id);
        return true;
      },
    );
  }, [paginatedData]);

  const displayCount = totalCount ?? issues.length;

  // Table layout uses the flat issue list for tree expansion. In board layout
  // the board owns its own tree expansion over the per-column issue union.
  const { childStatusMap, sessionsByIssue } = usePageIssueTrees(
    isTable ? issues : [],
    currentUser,
  );

  // Strip any unrecognised `?selected=…` values left by old links.
  useEffect(() => {
    const selected = searchParams.get("selected");
    if (selected && !LEGACY_SELECTED_VALUES.has(selected)) {
      setSearchParams(
        (prev) => {
          prev.delete("selected");
          return prev;
        },
        { replace: true },
      );
    }
  }, [searchParams, setSearchParams]);

  const handleLoadMore = useCallback(() => {
    if (hasNextPage && !isFetchingNextPage) fetchNextPage();
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);

  return (
    <div className={styles.page}>
      <IssuesView
        layout={layout}
        onLayoutChange={setLayout}
        issues={issues}
        childStatusMap={childStatusMap}
        sessionsByIssue={sessionsByIssue}
        isLoading={isLoading}
        baseFilters={boardBaseFilters}
        username={currentUser}
        filterRootId={framing.rootId}
        hasNextPage={hasNextPage ?? false}
        isFetchingNextPage={isFetchingNextPage ?? false}
        onLoadMore={handleLoadMore}
        eyebrow={formatEyebrow(framing.eyebrowPrefix, displayCount)}
        title={framing.title}
        filters={filters}
        setFilters={setFilters}
        definitions={definitions}
        filteredCount={issues.length}
        totalCount={displayCount}
        searchValue={searchValue}
        onSearchChange={handleSearchChange}
        onFilterMenuOpenChange={setAddMenuOpen}
      />
    </div>
  );
}
