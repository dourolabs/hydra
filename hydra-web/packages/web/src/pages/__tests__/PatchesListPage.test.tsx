// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, cleanup } from "@testing-library/react";
import React from "react";
import type { ListPatchesResponse, PatchSummaryRecord } from "@hydra/api";

// --- Mocks ---

const navigateMock = vi.fn();

let searchParamsString = "";
const setSearchParamsMock = vi.fn(
  (
    updater:
      | URLSearchParams
      | string
      | Record<string, string>
      | ((prev: URLSearchParams) => URLSearchParams),
  ) => {
    const prev = new URLSearchParams(searchParamsString);
    let next: URLSearchParams;
    if (typeof updater === "function") {
      next = updater(prev);
    } else if (updater instanceof URLSearchParams) {
      next = updater;
    } else if (typeof updater === "string") {
      next = new URLSearchParams(updater);
    } else {
      next = new URLSearchParams(updater);
    }
    searchParamsString = next.toString();
  },
);

vi.mock("react-router-dom", () => ({
  Link: ({
    to,
    children,
    className,
    onClick,
  }: {
    to: string;
    children: React.ReactNode;
    className?: string;
    onClick?: (e: React.MouseEvent) => void;
  }) => (
    <a href={to} className={className} onClick={onClick}>
      {children}
    </a>
  ),
  useNavigate: () => navigateMock,
  useSearchParams: () => {
    return [new URLSearchParams(searchParamsString), setSearchParamsMock] as const;
  },
}));

const usePaginatedPatchesMock = vi.fn();
const usePatchCountMock = vi.fn();

interface PaginatedPatchesState {
  pages: ListPatchesResponse[] | undefined;
  isLoading: boolean;
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
}

const paginatedState: PaginatedPatchesState = {
  pages: undefined,
  isLoading: false,
  hasNextPage: false,
  isFetchingNextPage: false,
};

const fetchNextPageMock = vi.fn();

const patchCountState: { count: number | undefined } = { count: undefined };

vi.mock("../../features/dashboard/usePaginatedPatches", () => ({
  usePaginatedPatches: (...args: unknown[]) => {
    usePaginatedPatchesMock(...args);
    return {
      data: paginatedState.pages ? { pages: paginatedState.pages } : undefined,
      isLoading: paginatedState.isLoading,
      fetchNextPage: fetchNextPageMock,
      hasNextPage: paginatedState.hasNextPage,
      isFetchingNextPage: paginatedState.isFetchingNextPage,
    };
  },
  usePatchCount: (...args: unknown[]) => {
    usePatchCountMock(...args);
    return { data: patchCountState.count };
  },
}));

// usePatchFilters loads option lists via React Query; stub to a no-op map.
vi.mock("../../features/patches/patchFilters", () => ({
  usePatchFilters: () => ({}),
}));

// Relation resolver issues `/v1/relations` via useQueries — stub it so the
// test can flip it into a loading state (PR-2 verifies that this loading
// state does NOT force the page into a skeleton when previous rows exist).
const relationsState: {
  patchIds: string[] | null;
  isLoading: boolean;
} = { patchIds: null, isLoading: false };

vi.mock("../../features/patches/useRelationFilteredPatchIds", () => ({
  useRelationFilteredPatchIds: () => ({
    patchIds: relationsState.patchIds,
    isLoading: relationsState.isLoading,
  }),
  RELATION_FILTER_IDS: ["relatedIssue", "relatedSession"],
}));

// FilterBar internally uses portals and pop-overs we don't exercise here.
vi.mock("../../features/filters", () => ({
  FilterBar: () => <div data-testid="filter-bar" />,
  applyFilters: <T,>(items: T[]) => items,
}));

vi.mock("@hydra/ui", () => ({
  Avatar: ({ name }: { name: string }) => <span data-testid="avatar">{name}</span>,
  Badge: ({ status }: { status: string }) => (
    <span data-testid="badge">{status}</span>
  ),
  Kbd: ({ children }: { children: React.ReactNode }) => (
    <kbd>{children}</kbd>
  ),
  Icons: new Proxy(
    {},
    {
      get: () => () => <span data-testid="icon" />,
    },
  ),
}));

vi.mock("../../utils/badgeStatus", () => ({
  normalizePatchStatus: (s: string) => s,
}));

vi.mock("../../features/patches/PatchRepoLink", () => ({
  PatchRepoLink: () => <span data-testid="patch-repo-link" />,
}));

vi.mock("../../features/related/RailRow", () => ({
  PatchRailRow: ({ record }: { record: PatchSummaryRecord }) => (
    <div data-testid={`patches-rail-row-${record.patch_id}`} />
  ),
}));

vi.mock("../../components/Runtime/Runtime", () => ({
  AgoTime: ({ iso }: { iso: string }) => <span>{iso}</span>,
}));

vi.mock("../../features/patches/view/PatchesView.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const useBreadcrumbsMock = vi.fn();
vi.mock("../../layout/useBreadcrumbs", () => ({
  useBreadcrumbs: (...args: unknown[]) => useBreadcrumbsMock(...args),
}));

// --- Import after mocks ---
const { PatchesListPage } = await import("../PatchesListPage");

// --- Helpers ---

function rec(id: string, title = `Patch ${id}`): PatchSummaryRecord {
  return {
    patch_id: id,
    version: 1n,
    timestamp: "2026-03-15T10:00:00.000Z",
    patch: {
      title,
      creator: "alice",
      status: "Open",
      review_summary: { count: 0, approved: false },
    },
  } as unknown as PatchSummaryRecord;
}

function setPatches(patches: PatchSummaryRecord[]) {
  paginatedState.pages = [{ patches }] as ListPatchesResponse[];
}

function reset() {
  paginatedState.pages = undefined;
  paginatedState.isLoading = false;
  paginatedState.hasNextPage = false;
  paginatedState.isFetchingNextPage = false;
  patchCountState.count = undefined;
  navigateMock.mockReset();
  fetchNextPageMock.mockReset();
  searchParamsString = "";
  setSearchParamsMock.mockClear();
  usePaginatedPatchesMock.mockReset();
  usePatchCountMock.mockReset();
  relationsState.patchIds = null;
  relationsState.isLoading = false;
}

describe("PatchesListPage relation-resolution loading persistence", () => {
  // PR-2: When the relation resolver re-runs (user changed a relation chip
  // value), the page must keep rendering the previously-loaded rows from
  // the list query — driven by `placeholderData: keepPreviousData` on
  // `usePaginatedPatches` / `usePatchCount` — instead of flashing the
  // skeleton or zero-state. The fix drops `|| relationsLoading` from the
  // `isLoading` prop forwarded to <PatchesView>.

  beforeEach(() => {
    reset();
    useBreadcrumbsMock.mockReset();
    cleanup();
  });

  it("keeps prior rows visible while relations are resolving", () => {
    searchParamsString = "relatedIssue=i-a";
    setPatches([rec("p-prev-1", "first prior patch"), rec("p-prev-2", "second prior patch")]);
    relationsState.patchIds = ["p-prev-1", "p-prev-2"];
    relationsState.isLoading = true;

    const { container } = render(<PatchesListPage />);

    // No loading / empty state — previous render persists.
    expect(screen.queryByText(/loading patches/i)).toBeNull();
    expect(screen.queryByText(/no patches match the current filters/i)).toBeNull();
    // Both prior rows still in the DOM.
    expect(container.textContent).toContain("first prior patch");
    expect(container.textContent).toContain("second prior patch");
  });

  it("renders the loading message only when there are no previous rows", () => {
    searchParamsString = "relatedIssue=i-a";
    paginatedState.isLoading = true;
    relationsState.isLoading = false;

    render(<PatchesListPage />);

    expect(screen.getByText(/loading patches/i)).toBeDefined();
  });
});
