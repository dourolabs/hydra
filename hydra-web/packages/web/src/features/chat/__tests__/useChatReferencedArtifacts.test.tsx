import { describe, it, expect, vi, beforeEach } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type {
  DocumentSummaryRecord,
  IssueSummaryRecord,
  ListDocumentsResponse,
  ListIssuesResponse,
  ListPatchesResponse,
  PatchSummaryRecord,
  SessionSummaryRecord,
} from "@hydra/api";

// --- Mocks ---

const mockListRelations = vi.fn();
const mockListIssues = vi.fn();
const mockListPatches = vi.fn();
const mockListDocuments = vi.fn();
const mockListSessions = vi.fn();

vi.mock("../../../api/client", () => ({
  apiClient: {
    listRelations: (...args: unknown[]) => mockListRelations(...args),
    listIssues: (...args: unknown[]) => mockListIssues(...args),
    listPatches: (...args: unknown[]) => mockListPatches(...args),
    listDocuments: (...args: unknown[]) => mockListDocuments(...args),
    listSessions: (...args: unknown[]) => mockListSessions(...args),
  },
}));

// --- Helpers ---

function makeWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return ({ children }: { children: React.ReactNode }) =>
    React.createElement(QueryClientProvider, { client: queryClient }, children);
}

function makeRelation(targetId: string, sourceId = "c-abc") {
  return { source_id: sourceId, target_id: targetId, rel_type: "refers-to" };
}

function makeIssue(id: string, title = `Issue ${id}`): IssueSummaryRecord {
  return {
    issue_id: id,
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    issue: {
      type: "task",
      title,
      description: "desc",
      creator: "alice",
      status: "open",
      progress: "",
      dependencies: [],
      patches: [],
      labels: [],
    },
  } as IssueSummaryRecord;
}

function makePatch(id: string, title = `Patch ${id}`): PatchSummaryRecord {
  return {
    patch_id: id,
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    patch: {
      title,
      status: "Open",
      is_automatic_backup: false,
      creator: "alice",
      review_summary: { count: 0, approved: false },
      service_repo_name: "test-repo",
    },
  } as PatchSummaryRecord;
}

function makeDocument(id: string, title = `Doc ${id}`): DocumentSummaryRecord {
  return {
    document_id: id,
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    document: {
      title,
      path: `docs/${id}.md`,
      deleted: false,
    },
  } as DocumentSummaryRecord;
}

function makeSession(
  sessionId: string,
  spawnedFrom: string | null,
  status: "running" | "pending" | "completed" = "running",
): SessionSummaryRecord {
  return {
    session_id: sessionId,
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    session: {
      prompt: "",
      spawned_from: spawnedFrom ?? null,
      creator: "alice",
      status,
    },
  } as SessionSummaryRecord;
}

function issuesPage(
  issues: IssueSummaryRecord[],
  nextCursor: string | null = null,
): ListIssuesResponse {
  return { issues, next_cursor: nextCursor } as ListIssuesResponse;
}

function patchesPage(
  patches: PatchSummaryRecord[],
  nextCursor: string | null = null,
): ListPatchesResponse {
  return { patches, next_cursor: nextCursor } as ListPatchesResponse;
}

function documentsPage(
  documents: DocumentSummaryRecord[],
  nextCursor: string | null = null,
): ListDocumentsResponse {
  return { documents, next_cursor: nextCursor } as ListDocumentsResponse;
}

// --- Import after mocks ---
const { useChatReferencedArtifacts } = await import(
  "../useChatReferencedArtifacts"
);

// --- Tests ---

describe("useChatReferencedArtifacts", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // Default: no sessions for issue ids — individual tests override.
    mockListSessions.mockResolvedValue({ sessions: [] });
  });

  it("buckets target ids by prefix and drops unknown prefixes", async () => {
    mockListRelations.mockResolvedValue({
      relations: [
        makeRelation("i-1"),
        makeRelation("p-1"),
        makeRelation("d-1"),
        makeRelation("x-junk"),
        makeRelation("i-2"),
      ],
    });
    mockListIssues.mockResolvedValue(
      issuesPage([makeIssue("i-1"), makeIssue("i-2")]),
    );
    mockListPatches.mockResolvedValue(patchesPage([makePatch("p-1")]));
    mockListDocuments.mockResolvedValue(documentsPage([makeDocument("d-1")]));

    const { result } = renderHook(() => useChatReferencedArtifacts("c-abc"), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => expect(result.current.isLoading).toBe(false));

    expect(result.current.issues.map((i) => i.issue_id)).toEqual([
      "i-1",
      "i-2",
    ]);
    expect(result.current.patches.map((p) => p.patch_id)).toEqual(["p-1"]);
    expect(result.current.documents.map((d) => d.document_id)).toEqual([
      "d-1",
    ]);

    expect(mockListRelations).toHaveBeenCalledWith({
      source_id: "c-abc",
      rel_type: "refers-to",
    });
    expect(mockListIssues).toHaveBeenCalledTimes(1);
    expect(mockListIssues).toHaveBeenCalledWith({
      ids: "i-1,i-2",
      limit: 25,
      cursor: null,
    });
    expect(mockListPatches).toHaveBeenCalledWith({
      ids: "p-1",
      limit: 25,
      cursor: null,
    });
    expect(mockListDocuments).toHaveBeenCalledTimes(1);
    expect(mockListDocuments).toHaveBeenCalledWith({
      ids: "d-1",
      limit: 25,
      cursor: null,
    });
    expect(result.current.error).toBeNull();
    expect(result.current.hasNextPage).toEqual({
      issues: false,
      patches: false,
      documents: false,
    });
  });

  it("renders the backend order verbatim across the flattened pages", async () => {
    // The backend returns issues sorted by updated_at desc — the hook must
    // render that order without re-sorting against the relation-id order.
    mockListRelations.mockResolvedValue({
      relations: [
        makeRelation("i-2"),
        makeRelation("i-3"),
        makeRelation("i-1"),
        makeRelation("p-b"),
        makeRelation("p-a"),
        makeRelation("d-y"),
        makeRelation("d-x"),
      ],
    });
    mockListIssues.mockResolvedValue(
      issuesPage([makeIssue("i-1"), makeIssue("i-2"), makeIssue("i-3")]),
    );
    mockListPatches.mockResolvedValue(
      patchesPage([makePatch("p-a"), makePatch("p-b")]),
    );
    mockListDocuments.mockResolvedValue(
      documentsPage([makeDocument("d-x"), makeDocument("d-y")]),
    );

    const { result } = renderHook(() => useChatReferencedArtifacts("c-abc"), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => expect(result.current.isLoading).toBe(false));

    expect(result.current.issues.map((i) => i.issue_id)).toEqual([
      "i-1",
      "i-2",
      "i-3",
    ]);
    expect(result.current.patches.map((p) => p.patch_id)).toEqual([
      "p-a",
      "p-b",
    ]);
    expect(result.current.documents.map((d) => d.document_id)).toEqual([
      "d-x",
      "d-y",
    ]);
  });

  it("does not cap the number of ids passed to listIssues (no 33-cap regression)", async () => {
    const relations = Array.from({ length: 40 }, (_, i) =>
      makeRelation(`i-${i}`),
    );
    mockListRelations.mockResolvedValue({ relations });

    const allIds = Array.from({ length: 40 }, (_, i) => `i-${i}`);
    mockListIssues.mockResolvedValue(
      issuesPage(allIds.map((id) => makeIssue(id))),
    );

    const { result } = renderHook(() => useChatReferencedArtifacts("c-abc"), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => expect(result.current.isLoading).toBe(false));

    expect(mockListIssues).toHaveBeenCalledWith({
      ids: allIds.join(","),
      limit: 25,
      cursor: null,
    });
    expect(result.current.issues).toHaveLength(40);
    expect(result.current.issues.map((i) => i.issue_id)).toEqual(allIds);
  });

  it("aggregates error from listRelations", async () => {
    mockListRelations.mockRejectedValue(new Error("relations failed"));

    const { result } = renderHook(() => useChatReferencedArtifacts("c-abc"), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => expect(result.current.error).not.toBeNull());
    expect(result.current.error).toBeInstanceOf(Error);
    expect(mockListIssues).not.toHaveBeenCalled();
    expect(mockListPatches).not.toHaveBeenCalled();
    expect(mockListDocuments).not.toHaveBeenCalled();
  });

  it("aggregates error from listIssues", async () => {
    mockListRelations.mockResolvedValue({
      relations: [makeRelation("i-1")],
    });
    mockListIssues.mockRejectedValue(new Error("issues failed"));

    const { result } = renderHook(() => useChatReferencedArtifacts("c-abc"), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => expect(result.current.error).not.toBeNull());
    expect(result.current.error).toBeInstanceOf(Error);
  });

  it("aggregates error from listPatches", async () => {
    mockListRelations.mockResolvedValue({
      relations: [makeRelation("p-1")],
    });
    mockListPatches.mockRejectedValue(new Error("patches failed"));

    const { result } = renderHook(() => useChatReferencedArtifacts("c-abc"), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => expect(result.current.error).not.toBeNull());
    expect(result.current.error).toBeInstanceOf(Error);
  });

  it("aggregates error from listDocuments", async () => {
    mockListRelations.mockResolvedValue({
      relations: [makeRelation("d-1"), makeRelation("d-2")],
    });
    mockListDocuments.mockRejectedValue(new Error("docs failed"));

    const { result } = renderHook(() => useChatReferencedArtifacts("c-abc"), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => expect(result.current.error).not.toBeNull());
    expect(result.current.error).toBeInstanceOf(Error);
  });

  it("reports isLoading=true while listRelations is pending", async () => {
    let resolveRelations: (val: { relations: never[] }) => void = () => {};
    mockListRelations.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveRelations = resolve;
        }),
    );

    const { result } = renderHook(() => useChatReferencedArtifacts("c-abc"), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => expect(result.current.isLoading).toBe(true));

    resolveRelations({ relations: [] });

    await waitFor(() => expect(result.current.isLoading).toBe(false));
  });

  it("reports isLoading=true while downstream queries are pending after relations resolve", async () => {
    mockListRelations.mockResolvedValue({
      relations: [
        makeRelation("i-1"),
        makeRelation("p-1"),
        makeRelation("d-1"),
      ],
    });

    let resolveIssues: (val: ListIssuesResponse) => void = () => {};
    let resolvePatches: (val: ListPatchesResponse) => void = () => {};
    let resolveDocuments: (val: ListDocumentsResponse) => void = () => {};

    mockListIssues.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveIssues = resolve;
        }),
    );
    mockListPatches.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolvePatches = resolve;
        }),
    );
    mockListDocuments.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveDocuments = resolve;
        }),
    );

    const { result } = renderHook(() => useChatReferencedArtifacts("c-abc"), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(mockListIssues).toHaveBeenCalled();
      expect(mockListPatches).toHaveBeenCalled();
      expect(mockListDocuments).toHaveBeenCalled();
    });
    expect(result.current.isLoading).toBe(true);

    resolveIssues(issuesPage([makeIssue("i-1")]));
    resolvePatches(patchesPage([makePatch("p-1")]));
    resolveDocuments(documentsPage([makeDocument("d-1")]));

    await waitFor(() => expect(result.current.isLoading).toBe(false));
    expect(result.current.error).toBeNull();
  });

  it("returns empty arrays without firing downstream queries when relations is empty", async () => {
    mockListRelations.mockResolvedValue({ relations: [] });

    const { result } = renderHook(() => useChatReferencedArtifacts("c-abc"), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => expect(result.current.isLoading).toBe(false));

    expect(result.current.issues).toEqual([]);
    expect(result.current.patches).toEqual([]);
    expect(result.current.documents).toEqual([]);
    expect(result.current.sessionsByIssue.size).toBe(0);
    expect(result.current.error).toBeNull();
    expect(mockListIssues).not.toHaveBeenCalled();
    expect(mockListPatches).not.toHaveBeenCalled();
    expect(mockListDocuments).not.toHaveBeenCalled();
    expect(mockListSessions).not.toHaveBeenCalled();
  });

  it("calls listSessions once with the comma-joined fetched issue ids", async () => {
    mockListRelations.mockResolvedValue({
      relations: [makeRelation("i-1"), makeRelation("i-2")],
    });
    mockListIssues.mockResolvedValue(
      issuesPage([makeIssue("i-1"), makeIssue("i-2")]),
    );
    mockListSessions.mockResolvedValue({ sessions: [] });

    const { result } = renderHook(() => useChatReferencedArtifacts("c-abc"), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => expect(result.current.isLoading).toBe(false));

    expect(mockListSessions).toHaveBeenCalledTimes(1);
    expect(mockListSessions).toHaveBeenCalledWith({
      spawned_from_ids: "i-1,i-2",
    });
  });

  it("groups sessions by spawned_from into sessionsByIssue", async () => {
    mockListRelations.mockResolvedValue({
      relations: [makeRelation("i-1"), makeRelation("i-2")],
    });
    mockListIssues.mockResolvedValue(
      issuesPage([makeIssue("i-1"), makeIssue("i-2")]),
    );
    mockListSessions.mockResolvedValue({
      sessions: [
        makeSession("s-a", "i-1", "running"),
        makeSession("s-b", "i-1", "pending"),
        makeSession("s-c", "i-2", "running"),
        makeSession("s-d", null, "running"),
      ],
    });

    const { result } = renderHook(() => useChatReferencedArtifacts("c-abc"), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => expect(result.current.isLoading).toBe(false));

    const map = result.current.sessionsByIssue;
    expect(map).toBeInstanceOf(Map);
    expect(map.size).toBe(2);
    expect(map.get("i-1")?.map((s) => s.session_id)).toEqual(["s-a", "s-b"]);
    expect(map.get("i-2")?.map((s) => s.session_id)).toEqual(["s-c"]);
  });

  it("does not call listSessions when there are no issue ids", async () => {
    mockListRelations.mockResolvedValue({
      relations: [makeRelation("p-1"), makeRelation("d-1")],
    });
    mockListPatches.mockResolvedValue(patchesPage([makePatch("p-1")]));
    mockListDocuments.mockResolvedValue(documentsPage([makeDocument("d-1")]));

    const { result } = renderHook(() => useChatReferencedArtifacts("c-abc"), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => expect(result.current.isLoading).toBe(false));

    expect(mockListSessions).not.toHaveBeenCalled();
    expect(result.current.sessionsByIssue.size).toBe(0);
  });

  it("aggregates error from listSessions", async () => {
    mockListRelations.mockResolvedValue({
      relations: [makeRelation("i-1")],
    });
    mockListIssues.mockResolvedValue(issuesPage([makeIssue("i-1")]));
    mockListSessions.mockRejectedValue(new Error("sessions failed"));

    const { result } = renderHook(() => useChatReferencedArtifacts("c-abc"), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => expect(result.current.error).not.toBeNull());
    expect(result.current.error).toBeInstanceOf(Error);
  });

  it("exposes hasNextPage.issues=true and fetches the next page on demand, appending results in returned order", async () => {
    mockListRelations.mockResolvedValue({
      relations: [makeRelation("i-1"), makeRelation("i-2")],
    });
    mockListIssues
      .mockResolvedValueOnce(issuesPage([makeIssue("i-1")], "cursor-1"))
      .mockResolvedValueOnce(issuesPage([makeIssue("i-2")]));

    const { result } = renderHook(() => useChatReferencedArtifacts("c-abc"), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => expect(result.current.isLoading).toBe(false));
    expect(result.current.issues.map((i) => i.issue_id)).toEqual(["i-1"]);
    expect(result.current.hasNextPage.issues).toBe(true);
    expect(mockListIssues).toHaveBeenCalledTimes(1);
    expect(mockListIssues).toHaveBeenNthCalledWith(1, {
      ids: "i-1,i-2",
      limit: 25,
      cursor: null,
    });

    act(() => {
      result.current.fetchNextPage.issues();
    });

    await waitFor(() =>
      expect(result.current.issues.map((i) => i.issue_id)).toEqual([
        "i-1",
        "i-2",
      ]),
    );
    expect(result.current.hasNextPage.issues).toBe(false);
    expect(mockListIssues).toHaveBeenCalledTimes(2);
    expect(mockListIssues).toHaveBeenNthCalledWith(2, {
      ids: "i-1,i-2",
      limit: 25,
      cursor: "cursor-1",
    });
  });

  it("paginates beyond 33 issues and returns every id after enough fetches", async () => {
    const relations = Array.from({ length: 40 }, (_, i) =>
      makeRelation(`i-${i}`),
    );
    mockListRelations.mockResolvedValue({ relations });

    const firstPage = Array.from({ length: 25 }, (_, i) => makeIssue(`i-${i}`));
    const secondPage = Array.from({ length: 15 }, (_, i) =>
      makeIssue(`i-${i + 25}`),
    );

    mockListIssues
      .mockResolvedValueOnce(issuesPage(firstPage, "cursor-page-2"))
      .mockResolvedValueOnce(issuesPage(secondPage));

    const { result } = renderHook(() => useChatReferencedArtifacts("c-abc"), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => expect(result.current.isLoading).toBe(false));

    expect(result.current.issues).toHaveLength(25);
    expect(result.current.hasNextPage.issues).toBe(true);

    act(() => {
      result.current.fetchNextPage.issues();
    });

    await waitFor(() => expect(result.current.issues).toHaveLength(40));
    expect(result.current.issues.map((i) => i.issue_id)).toEqual(
      Array.from({ length: 40 }, (_, i) => `i-${i}`),
    );
    expect(result.current.hasNextPage.issues).toBe(false);
  });

  it("exposes hasNextPage.patches=true and fetches the next page on demand", async () => {
    mockListRelations.mockResolvedValue({
      relations: [makeRelation("p-1"), makeRelation("p-2")],
    });
    mockListPatches
      .mockResolvedValueOnce(patchesPage([makePatch("p-1")], "p-cursor"))
      .mockResolvedValueOnce(patchesPage([makePatch("p-2")]));

    const { result } = renderHook(() => useChatReferencedArtifacts("c-abc"), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => expect(result.current.isLoading).toBe(false));
    expect(result.current.hasNextPage.patches).toBe(true);

    act(() => {
      result.current.fetchNextPage.patches();
    });

    await waitFor(() =>
      expect(result.current.patches.map((p) => p.patch_id)).toEqual([
        "p-1",
        "p-2",
      ]),
    );
    expect(mockListPatches).toHaveBeenNthCalledWith(2, {
      ids: "p-1,p-2",
      limit: 25,
      cursor: "p-cursor",
    });
  });

  it("exposes hasNextPage.documents=true and fetches the next page on demand", async () => {
    mockListRelations.mockResolvedValue({
      relations: [makeRelation("d-1"), makeRelation("d-2")],
    });
    mockListDocuments
      .mockResolvedValueOnce(documentsPage([makeDocument("d-1")], "d-cursor"))
      .mockResolvedValueOnce(documentsPage([makeDocument("d-2")]));

    const { result } = renderHook(() => useChatReferencedArtifacts("c-abc"), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => expect(result.current.isLoading).toBe(false));
    expect(result.current.hasNextPage.documents).toBe(true);

    act(() => {
      result.current.fetchNextPage.documents();
    });

    await waitFor(() =>
      expect(result.current.documents.map((d) => d.document_id)).toEqual([
        "d-1",
        "d-2",
      ]),
    );
    expect(mockListDocuments).toHaveBeenNthCalledWith(2, {
      ids: "d-1,d-2",
      limit: 25,
      cursor: "d-cursor",
    });
  });

  it("does not leak the previous conversation's referenced artifacts when conversationId changes to one with no references", async () => {
    // Chat c-a has linked issue/patch/document; chat c-b has none.
    // Re-rendering the hook with c-b must immediately show empty arrays — no
    // stale placeholder data from c-a's cache.
    mockListRelations.mockImplementation(
      ({ source_id }: { source_id: string }) => {
        if (source_id === "c-a") {
          return Promise.resolve({
            relations: [
              makeRelation("i-1", "c-a"),
              makeRelation("p-1", "c-a"),
              makeRelation("d-1", "c-a"),
            ],
          });
        }
        return Promise.resolve({ relations: [] });
      },
    );
    mockListIssues.mockResolvedValue(issuesPage([makeIssue("i-1")]));
    mockListPatches.mockResolvedValue(patchesPage([makePatch("p-1")]));
    mockListDocuments.mockResolvedValue(documentsPage([makeDocument("d-1")]));

    // Share one QueryClient across renders so c-a's cached data is available
    // when we switch to c-b — this is what makes the placeholder fire in prod.
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const wrapper = ({ children }: { children: React.ReactNode }) =>
      React.createElement(QueryClientProvider, { client: queryClient }, children);

    const { result, rerender } = renderHook(
      ({ id }: { id: string }) => useChatReferencedArtifacts(id),
      { wrapper, initialProps: { id: "c-a" } },
    );

    await waitFor(() => {
      expect(result.current.issues.map((i) => i.issue_id)).toEqual(["i-1"]);
      expect(result.current.patches.map((p) => p.patch_id)).toEqual(["p-1"]);
      expect(result.current.documents.map((d) => d.document_id)).toEqual(["d-1"]);
    });

    rerender({ id: "c-b" });

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });

    expect(result.current.issues).toEqual([]);
    expect(result.current.patches).toEqual([]);
    expect(result.current.documents).toEqual([]);
    expect(result.current.sessionsByIssue.size).toBe(0);
  });

  it("re-fires listSessions for newly-loaded issue ids after fetchNextPage.issues", async () => {
    mockListRelations.mockResolvedValue({
      relations: [makeRelation("i-1"), makeRelation("i-2")],
    });
    mockListIssues
      .mockResolvedValueOnce(issuesPage([makeIssue("i-1")], "cur"))
      .mockResolvedValueOnce(issuesPage([makeIssue("i-2")]));
    mockListSessions.mockResolvedValue({ sessions: [] });

    const { result } = renderHook(() => useChatReferencedArtifacts("c-abc"), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => expect(result.current.isLoading).toBe(false));
    expect(mockListSessions).toHaveBeenLastCalledWith({
      spawned_from_ids: "i-1",
    });

    act(() => {
      result.current.fetchNextPage.issues();
    });

    await waitFor(() =>
      expect(mockListSessions).toHaveBeenLastCalledWith({
        spawned_from_ids: "i-1,i-2",
      }),
    );
  });
});
