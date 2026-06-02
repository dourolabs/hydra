import { useCallback, useEffect, useMemo, useState } from "react";
import { useSearchParams } from "react-router-dom";
import type { IssueStatus, IssueType } from "@hydra/api";
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
import styles from "./IssuesListPage.module.css";

// Map the legacy `?selected=...` shortcut onto the explicit filter params so
// e2e tests and bookmarked URLs continue to work after the sidebar switched
// to the new scheme. These URL params still drive eyebrow / breadcrumb /
// board-mode query state — even though the table-mode FilterBar is now
// purely client-side, the page-level "All / My / Assigned" framing remains
// URL-pinned for sharability.
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

interface FilterState {
  status: IssueStatus | null;
  type: IssueType | null;
  creator: string;
  assignee: string;
  label: string;
}

function parseStatus(value: string | null): IssueStatus | null {
  if (!value) return null;
  // Allow either dash or underscore for the in-progress legacy form.
  if (value === "in_progress") return "in-progress";
  return value as IssueStatus;
}

function parseType(value: string | null): IssueType | null {
  return value ? (value as IssueType) : null;
}

function resolveFilters(
  searchParams: URLSearchParams,
  currentUser: string,
  currentPrincipalPath: string | null,
): FilterState {
  // Explicit filter params take precedence over the legacy `selected=` shortcut.
  const hasExplicit =
    searchParams.has("status") ||
    searchParams.has("type") ||
    searchParams.has("creator") ||
    searchParams.has("assignee") ||
    searchParams.has("label");

  if (hasExplicit) {
    return {
      status: parseStatus(searchParams.get("status")),
      type: parseType(searchParams.get("type")),
      creator: searchParams.get("creator") ?? "",
      assignee: searchParams.get("assignee") ?? "",
      label: searchParams.get("label") ?? "",
    };
  }

  const selected = searchParams.get("selected");
  if (selected && LEGACY_SELECTED_VALUES.has(selected)) {
    if (selected === "your-issues") {
      return { status: null, type: null, creator: currentUser, assignee: "", label: "" };
    }
    if (selected === "assigned") {
      // Phase 4b: assignee filter is on the wire as a Principal path
      // (`users/alice` / `agents/swe`). If the logged-in actor doesn't have a
      // Principal form (session / service / etc.), fall through to "no filter"
      // rather than producing a malformed query that the server would 400 on.
      return {
        status: null,
        type: null,
        creator: "",
        assignee: currentPrincipalPath ?? "",
        label: "",
      };
    }
    if (selected === "in_progress") {
      return { status: "in-progress", type: null, creator: "", assignee: "", label: "" };
    }
    // "all" — explicit no-filter via legacy URL.
  }

  // Default: All issues. The "My issues" view is reachable via the sidebar's
  // Issues link, which injects `?creator=<currentUser>` explicitly. A bare
  // `/` means "show everything" so clicking the All issues link from a
  // filtered view always clears the filters.
  return { status: null, type: null, creator: "", assignee: "", label: "" };
}

function buildServerFilters(state: FilterState): IssueFilters {
  const filters: IssueFilters = {};
  if (state.status) filters.status = state.status;
  if (state.type) filters.type = state.type;
  if (state.creator) filters.creator = state.creator;
  if (state.assignee) filters.assignee = state.assignee;
  if (state.label) filters.labels = state.label;
  return filters;
}

function describeFilters(
  state: FilterState,
  currentUser: string,
  currentPrincipalPath: string | null,
): { rootId: string; title: string; eyebrowPrefix: string } {
  const onlyCreatorIsMe =
    !!currentUser &&
    state.creator === currentUser &&
    !state.status &&
    !state.type &&
    !state.assignee &&
    !state.label;
  if (onlyCreatorIsMe) {
    return { rootId: "your-issues", title: "My issues", eyebrowPrefix: "MINE" };
  }

  const onlyAssigneeIsMe =
    !!currentPrincipalPath &&
    state.assignee === currentPrincipalPath &&
    !state.status &&
    !state.type &&
    !state.creator &&
    !state.label;
  if (onlyAssigneeIsMe) {
    return { rootId: "assigned", title: "Assigned to me", eyebrowPrefix: "ASSIGNED" };
  }

  const onlyInProgress =
    state.status === "in-progress" &&
    !state.type &&
    !state.creator &&
    !state.assignee &&
    !state.label;
  if (onlyInProgress) {
    return { rootId: "in_progress", title: "In progress", eyebrowPrefix: "IN PROGRESS" };
  }

  const hasAnyFilter =
    !!state.status ||
    !!state.type ||
    !!state.creator ||
    !!state.assignee ||
    !!state.label;
  if (!hasAnyFilter) {
    return { rootId: "all", title: "All issues", eyebrowPrefix: "ALL" };
  }

  return { rootId: "filtered", title: "Issues", eyebrowPrefix: "FILTERED" };
}

function formatEyebrow(prefix: string, count: number): string {
  const n = count === 1 ? "1 ISSUE" : `${count} ISSUES`;
  return `${prefix} · ${n}`;
}

export function IssuesListPage() {
  const [searchParams, setSearchParams] = useSearchParams();
  const { user } = useAuth();
  const currentUser = user ? actorDisplayName(user.actor) : "";
  const currentPrincipalPath = user ? actorPrincipalPath(user.actor) : null;

  const filterState = useMemo(
    () => resolveFilters(searchParams, currentUser, currentPrincipalPath),
    [searchParams, currentUser, currentPrincipalPath],
  );

  const { rootId, title, eyebrowPrefix } = describeFilters(
    filterState,
    currentUser,
    currentPrincipalPath,
  );
  useBreadcrumbs([{ label: "Workspace", to: "/" }], title);

  const [layout, setLayout] = useState<IssuesLayout>(readLayout);
  useEffect(() => {
    writeLayout(layout);
  }, [layout]);
  const isTable = layout === "table";

  // Table mode now loads the unfiltered page (the FilterBar narrows
  // client-side over the loaded subset). Board mode still consumes the
  // URL-derived filters via `baseFilters`, both because it's out of scope
  // for this PR and because per-column server queries need a baseline.
  const boardBaseFilters = useMemo(
    () => buildServerFilters(filterState),
    [filterState],
  );
  const tableServerFilters = useMemo<IssueFilters>(() => ({}), []);

  const {
    data: paginatedData,
    isLoading,
    fetchNextPage,
    hasNextPage,
    isFetchingNextPage,
  } = usePaginatedIssues(tableServerFilters, isTable);

  const { data: totalCount } = useIssueCount(tableServerFilters);

  const issues = useMemo(() => {
    const seen = new Set<string>();
    return (paginatedData?.pages.flatMap((page) => page.issues) ?? []).filter((issue) => {
      if (seen.has(issue.issue_id)) return false;
      seen.add(issue.issue_id);
      return true;
    });
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
        filterRootId={rootId}
        hasNextPage={hasNextPage ?? false}
        isFetchingNextPage={isFetchingNextPage ?? false}
        onLoadMore={handleLoadMore}
        eyebrow={formatEyebrow(eyebrowPrefix, displayCount)}
        title={title}
      />
    </div>
  );
}
