import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type {
  DocumentSummaryRecord,
  IssueSummaryRecord,
  PatchSummaryRecord,
} from "@hydra/api";

// --- Mocks ---

const mockListRelations = vi.fn();
const mockListIssues = vi.fn();
const mockListPatches = vi.fn();
const mockListDocuments = vi.fn();

vi.mock("../../../api/client", () => ({
  apiClient: {
    listRelations: (...args: unknown[]) => mockListRelations(...args),
    listIssues: (...args: unknown[]) => mockListIssues(...args),
    listPatches: (...args: unknown[]) => mockListPatches(...args),
    listDocuments: (...args: unknown[]) => mockListDocuments(...args),
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
  return { source_id: sourceId, target_id: targetId, rel_type: "refers_to" };
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

// --- Import after mocks ---
const { useChatReferencedArtifacts } = await import(
  "../useChatReferencedArtifacts"
);

// --- Tests ---

describe("useChatReferencedArtifacts", () => {
  beforeEach(() => {
    vi.clearAllMocks();
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
    mockListIssues.mockResolvedValue({
      issues: [makeIssue("i-1"), makeIssue("i-2")],
    });
    mockListPatches.mockResolvedValue({
      patches: [makePatch("p-1")],
    });
    mockListDocuments.mockResolvedValue({
      documents: [makeDocument("d-1")],
    });

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
      rel_type: "refers_to",
    });
    expect(mockListIssues).toHaveBeenCalledTimes(1);
    expect(mockListIssues).toHaveBeenCalledWith({ ids: "i-1,i-2", limit: 2 });
    expect(mockListPatches).toHaveBeenCalledWith({ ids: "p-1", limit: 1 });
    expect(mockListDocuments).toHaveBeenCalledTimes(1);
    expect(mockListDocuments).toHaveBeenCalledWith({ ids: "d-1", limit: 1 });
    expect(result.current.error).toBeNull();
  });

  it("preserves the order returned by listRelations within each bucket", async () => {
    // listRelations returns ids in non-sorted order (backend is newest-first).
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
    // Underlying batch fetches return rows in a different order than the
    // relation order — the hook should re-order back to relation order.
    mockListIssues.mockResolvedValue({
      issues: [makeIssue("i-1"), makeIssue("i-2"), makeIssue("i-3")],
    });
    mockListPatches.mockResolvedValue({
      patches: [makePatch("p-a"), makePatch("p-b")],
    });
    mockListDocuments.mockResolvedValue({
      documents: [makeDocument("d-x"), makeDocument("d-y")],
    });

    const { result } = renderHook(() => useChatReferencedArtifacts("c-abc"), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => expect(result.current.isLoading).toBe(false));

    expect(result.current.issues.map((i) => i.issue_id)).toEqual([
      "i-2",
      "i-3",
      "i-1",
    ]);
    expect(result.current.patches.map((p) => p.patch_id)).toEqual([
      "p-b",
      "p-a",
    ]);
    expect(result.current.documents.map((d) => d.document_id)).toEqual([
      "d-y",
      "d-x",
    ]);
  });

  it("caps each bucket at 33 ids", async () => {
    const relations = Array.from({ length: 40 }, (_, i) =>
      makeRelation(`i-${i}`),
    );
    mockListRelations.mockResolvedValue({ relations });

    const expectedIds = Array.from({ length: 33 }, (_, i) => `i-${i}`);
    mockListIssues.mockResolvedValue({
      issues: expectedIds.map((id) => makeIssue(id)),
    });

    const { result } = renderHook(() => useChatReferencedArtifacts("c-abc"), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => expect(result.current.isLoading).toBe(false));

    expect(mockListIssues).toHaveBeenCalledWith({
      ids: expectedIds.join(","),
      limit: 33,
    });
    expect(result.current.issues).toHaveLength(33);
    expect(result.current.issues.map((i) => i.issue_id)).toEqual(expectedIds);
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

    let resolveIssues: (val: { issues: IssueSummaryRecord[] }) => void =
      () => {};
    let resolvePatches: (val: { patches: PatchSummaryRecord[] }) => void =
      () => {};
    let resolveDocuments: (val: {
      documents: DocumentSummaryRecord[];
    }) => void = () => {};

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

    // Wait until listRelations has resolved and the downstream queries have
    // been kicked off. isLoading should remain true while they are pending.
    await waitFor(() => {
      expect(mockListIssues).toHaveBeenCalled();
      expect(mockListPatches).toHaveBeenCalled();
      expect(mockListDocuments).toHaveBeenCalled();
    });
    expect(result.current.isLoading).toBe(true);

    resolveIssues({ issues: [makeIssue("i-1")] });
    resolvePatches({ patches: [makePatch("p-1")] });
    resolveDocuments({ documents: [makeDocument("d-1")] });

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
    expect(result.current.error).toBeNull();
    expect(mockListIssues).not.toHaveBeenCalled();
    expect(mockListPatches).not.toHaveBeenCalled();
    expect(mockListDocuments).not.toHaveBeenCalled();
  });
});
