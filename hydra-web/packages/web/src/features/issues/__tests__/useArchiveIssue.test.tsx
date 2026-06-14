// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor, act } from "@testing-library/react";
import {
  QueryClient,
  QueryClientProvider,
  type InfiniteData,
} from "@tanstack/react-query";
import React from "react";
import type {
  IssueSummaryRecord,
  ListIssuesResponse,
} from "@hydra/api";
import { ToastContext } from "../../toast/toast-state";

const mockDeleteIssue = vi.fn();

vi.mock("../../../api/client", () => ({
  apiClient: {
    deleteIssue: (...args: unknown[]) => mockDeleteIssue(...args),
  },
}));

const addToast = vi.fn();

function makeWrapper(queryClient: QueryClient) {
  return ({ children }: { children: React.ReactNode }) =>
    React.createElement(
      ToastContext.Provider,
      { value: { addToast } },
      React.createElement(
        QueryClientProvider,
        { client: queryClient },
        children,
      ),
    );
}

function issue(id: string): IssueSummaryRecord {
  return {
    issue_id: id,
    version: 1n,
    timestamp: "2026-05-01T00:00:00.000Z",
    issue: {
      type: "task",
      title: id,
      description: "",
      creator: "alice",
      progress: "",
      status: {
        key: "open",
        label: "open",
        color: "#3498db",
        position: 0,
        unblocks_parents: false,
        unblocks_dependents: false,
        cascades_to_children: false,
      },
      assignee: null,
      session_settings: null,
      dependencies: [],
      patches: [],
      project_id: "j-defaul",
    },
    creation_time: "2026-05-01T00:00:00.000Z",
  } as unknown as IssueSummaryRecord;
}

function page(issues: IssueSummaryRecord[]): ListIssuesResponse {
  return { issues, next_cursor: null } as ListIssuesResponse;
}

const { useArchiveIssue } = await import("../useArchiveIssue");

describe("useArchiveIssue cache updates across paginatedIssues shapes", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockDeleteIssue.mockResolvedValue(undefined);
  });

  // Regression: the board view caches the bucketed list response as a single
  // `ListIssuesResponse` (not `InfiniteData`, not an array), so the updater
  // hit `old.pages.map(...)` and threw "Cannot read properties of undefined
  // (reading 'map')" when archive was clicked from a row that lived in the
  // board cache.
  it("drops the row from the board bulk cache (single ListIssuesResponse) without throwing", async () => {
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const bulkKey = [
      "paginatedIssues",
      {},
      "board-bulk",
      "project_status_time_desc",
    ] as const;
    queryClient.setQueryData<ListIssuesResponse>(
      bulkKey,
      page([issue("i-a"), issue("i-b")]),
    );

    const { result } = renderHook(() => useArchiveIssue("i-a"), {
      wrapper: makeWrapper(queryClient),
    });

    await act(async () => {
      result.current.archive();
    });

    await waitFor(() => {
      expect(mockDeleteIssue).toHaveBeenCalledWith("i-a");
    });

    const updated = queryClient.getQueryData<ListIssuesResponse>(bulkKey);
    expect(updated?.issues.map((r) => r.issue_id)).toEqual(["i-b"]);
    // The error path would have surfaced the TypeError as an error toast.
    expect(addToast).not.toHaveBeenCalledWith(
      expect.stringContaining("undefined"),
      "error",
    );
  });

  it("drops the row from a per-cell expanded board cache (ListIssuesResponse[])", async () => {
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const cellKey = [
      "paginatedIssues",
      { project_id: "j-defaul", status: "open" },
      "depth",
      2,
    ] as const;
    queryClient.setQueryData<ListIssuesResponse[]>(cellKey, [
      page([issue("i-a"), issue("i-b")]),
    ]);

    const { result } = renderHook(() => useArchiveIssue("i-a"), {
      wrapper: makeWrapper(queryClient),
    });

    await act(async () => {
      result.current.archive();
    });

    await waitFor(() => {
      expect(mockDeleteIssue).toHaveBeenCalledWith("i-a");
    });

    const updated = queryClient.getQueryData<ListIssuesResponse[]>(cellKey);
    expect(updated?.[0].issues.map((r) => r.issue_id)).toEqual(["i-b"]);
  });

  it("drops the row from the table-view infinite cache (InfiniteData<ListIssuesResponse>)", async () => {
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const infiniteKey = [
      "paginatedIssues",
      {},
      "sort",
      "project_status_time_desc",
    ] as const;
    queryClient.setQueryData<InfiniteData<ListIssuesResponse>>(infiniteKey, {
      pages: [page([issue("i-a"), issue("i-b")])],
      pageParams: [undefined],
    });

    const { result } = renderHook(() => useArchiveIssue("i-a"), {
      wrapper: makeWrapper(queryClient),
    });

    await act(async () => {
      result.current.archive();
    });

    await waitFor(() => {
      expect(mockDeleteIssue).toHaveBeenCalledWith("i-a");
    });

    const updated =
      queryClient.getQueryData<InfiniteData<ListIssuesResponse>>(infiniteKey);
    expect(updated?.pages[0].issues.map((r) => r.issue_id)).toEqual(["i-b"]);
  });
});
