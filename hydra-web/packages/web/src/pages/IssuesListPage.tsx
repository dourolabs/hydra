import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";
import type { IssueStatus, IssueType } from "@hydra/api";
import {
  usePaginatedIssues,
  useIssueCount,
  type IssueFilters,
} from "../features/issues/usePaginatedIssues";
import { useAuth } from "../features/auth/useAuth";
import { useAgents } from "../hooks/useAgents";
import { actorDisplayName } from "../api/auth";
import { IssuesView } from "../features/issues/view/IssuesView";
import { usePageIssueTrees } from "../features/dashboard/usePageIssueTrees";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import styles from "./IssuesListPage.module.css";

// Map the legacy `?selected=...` shortcut onto the explicit filter params so
// e2e tests and bookmarked URLs continue to work after the sidebar switched
// to the new scheme.
const LEGACY_SELECTED_VALUES = new Set([
  "your-issues",
  "assigned",
  "all",
  "in_progress",
]);

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
      return { status: null, type: null, creator: "", assignee: currentUser, label: "" };
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

function buildServerFilters(state: FilterState, searchQuery: string): IssueFilters {
  const filters: IssueFilters = {};
  if (searchQuery) filters.q = searchQuery;
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
    !!currentUser &&
    state.assignee === currentUser &&
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

  const filterState = useMemo(
    () => resolveFilters(searchParams, currentUser),
    [searchParams, currentUser],
  );

  const { rootId, title, eyebrowPrefix } = describeFilters(filterState, currentUser);
  useBreadcrumbs([{ label: "Workspace", to: "/" }], title);

  const [searchValue, setSearchValue] = useState(searchParams.get("q") ?? "");
  const [searchQuery, setSearchQuery] = useState(searchParams.get("q") ?? "");
  const debounceRef = useRef<ReturnType<typeof setTimeout>>(undefined);

  const handleSearchChange = useCallback((value: string) => {
    setSearchValue(value);
    clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => {
      setSearchQuery(value);
    }, 300);
  }, []);

  useEffect(() => {
    return () => clearTimeout(debounceRef.current);
  }, []);

  // Helper that updates a single filter param in the URL. Setting an empty
  // value removes the param; setting any other value also clears the legacy
  // `selected=` shortcut so the two schemes never coexist in the URL.
  const setParam = useCallback(
    (key: string, value: string | null) => {
      setSearchParams(
        (prev) => {
          const next = new URLSearchParams(prev);
          if (value) {
            next.set(key, value);
          } else {
            next.delete(key);
          }
          next.delete("selected");
          return next;
        },
        { replace: false },
      );
    },
    [setSearchParams],
  );

  const handleStatusChange = useCallback(
    (status: IssueStatus | null) => setParam("status", status),
    [setParam],
  );
  const handleTypeChange = useCallback(
    (type: IssueType | null) => setParam("type", type),
    [setParam],
  );
  const handleCreatorChange = useCallback(
    (creator: string) => setParam("creator", creator || null),
    [setParam],
  );
  const handleAssigneeChange = useCallback(
    (assignee: string) => setParam("assignee", assignee || null),
    [setParam],
  );

  const { data: agents } = useAgents();
  const userOptions = useMemo(() => {
    const names = new Set<string>();
    for (const a of agents ?? []) names.add(a.name);
    if (currentUser) names.add(currentUser);
    return Array.from(names).sort();
  }, [agents, currentUser]);

  const serverFilters = useMemo(
    () => buildServerFilters(filterState, searchQuery),
    [filterState, searchQuery],
  );

  const {
    data: paginatedData,
    isLoading,
    fetchNextPage,
    hasNextPage,
    isFetchingNextPage,
  } = usePaginatedIssues(serverFilters, true);

  const { data: totalCount } = useIssueCount(serverFilters);

  const issues = useMemo(() => {
    const seen = new Set<string>();
    return (paginatedData?.pages.flatMap((page) => page.issues) ?? []).filter((issue) => {
      if (seen.has(issue.issue_id)) return false;
      seen.add(issue.issue_id);
      return true;
    });
  }, [paginatedData]);

  const displayCount = totalCount ?? issues.length;

  const { childStatusMap, sessionsByIssue } = usePageIssueTrees(issues, currentUser);

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
        issues={issues}
        childStatusMap={childStatusMap}
        sessionsByIssue={sessionsByIssue}
        isLoading={isLoading}
        filterRootId={rootId}
        hasNextPage={hasNextPage ?? false}
        isFetchingNextPage={isFetchingNextPage ?? false}
        onLoadMore={handleLoadMore}
        searchValue={searchValue}
        onSearchChange={handleSearchChange}
        selectedStatus={filterState.status}
        onStatusChange={handleStatusChange}
        selectedType={filterState.type}
        onTypeChange={handleTypeChange}
        selectedCreator={filterState.creator}
        onCreatorChange={handleCreatorChange}
        selectedAssignee={filterState.assignee}
        onAssigneeChange={handleAssigneeChange}
        userOptions={userOptions}
        eyebrow={formatEyebrow(eyebrowPrefix, displayCount)}
        title={title}
      />
    </div>
  );
}
