// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type { Filter } from "../../filters";

const mockListRelations = vi.fn();

vi.mock("../../../api/client", () => ({
  apiClient: {
    listRelations: (...args: unknown[]) => mockListRelations(...args),
    getSession: () => Promise.resolve(null),
  },
}));

function makeWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return ({ children }: { children: React.ReactNode }) =>
    React.createElement(QueryClientProvider, { client: queryClient }, children);
}

const { useRelationFilteredIssueIds, capRelationIds, MAX_IDS_CSV_LEN } =
  await import("../useRelationFilteredIssueIds");

describe("capRelationIds", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    warnSpy.mockRestore();
  });

  it("returns the input unchanged when the set fits within the cap", () => {
    const ids = Array.from({ length: MAX_IDS_CSV_LEN }, (_, i) => `i-${i}`);
    const out = capRelationIds(ids);
    expect(out).toBe(ids);
    expect(warnSpy).not.toHaveBeenCalled();
  });

  it("truncates to MAX_IDS_CSV_LEN and warns when the set exceeds the cap", () => {
    const ids = Array.from(
      { length: MAX_IDS_CSV_LEN + 25 },
      (_, i) => `i-${i}`,
    );
    const out = capRelationIds(ids);
    expect(out.length).toBe(MAX_IDS_CSV_LEN);
    expect(out[0]).toBe("i-0");
    expect(out[MAX_IDS_CSV_LEN - 1]).toBe(`i-${MAX_IDS_CSV_LEN - 1}`);
    expect(warnSpy).toHaveBeenCalledTimes(1);
    const msg = String(warnSpy.mock.calls[0][0]);
    expect(msg).toContain(`${MAX_IDS_CSV_LEN + 25}`);
    expect(msg).toContain(`${MAX_IDS_CSV_LEN}`);
  });

  it("MAX_IDS_CSV_LEN matches the documented SearchIssuesQuery.ids cap", () => {
    expect(MAX_IDS_CSV_LEN).toBe(100);
  });
});

describe("useRelationFilteredIssueIds truncation", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    vi.clearAllMocks();
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    warnSpy.mockRestore();
  });

  it("caps the resolved issueIds to 100 when the relation lookup returns more", async () => {
    // Single relation filter on `relatedPatch` selecting one patch. The
    // server returns 150 relations matching that target — the hook must
    // intersect (trivially, since there's only one set) and cap to 100.
    const tooMany = Array.from({ length: 150 }, (_, i) => ({
      source_id: `i-${i.toString().padStart(3, "0")}`,
      target_id: "p-1",
    }));
    mockListRelations.mockResolvedValue({ relations: tooMany });

    const filters: Filter[] = [
      { _uid: "u1", id: "relatedPatch", op: "in", values: ["p-1"] },
    ];

    const { result } = renderHook(() => useRelationFilteredIssueIds(filters), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
      expect(result.current.issueIds).not.toBeNull();
    });

    expect(result.current.issueIds?.length).toBe(MAX_IDS_CSV_LEN);
    expect(warnSpy).toHaveBeenCalledTimes(1);
  });

  it("does not warn or truncate when the intersected set is small", async () => {
    mockListRelations.mockResolvedValue({
      relations: [
        { source_id: "i-1", target_id: "p-1" },
        { source_id: "i-2", target_id: "p-1" },
      ],
    });

    const filters: Filter[] = [
      { _uid: "u1", id: "relatedPatch", op: "in", values: ["p-1"] },
    ];

    const { result } = renderHook(() => useRelationFilteredIssueIds(filters), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
      expect(result.current.issueIds).not.toBeNull();
    });

    expect(result.current.issueIds?.length).toBe(2);
    expect(warnSpy).not.toHaveBeenCalled();
  });
});
