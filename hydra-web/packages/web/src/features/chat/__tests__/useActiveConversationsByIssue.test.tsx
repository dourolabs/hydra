// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type { ConversationSummary, ListConversationsResponse } from "@hydra/api";

const mockListConversations = vi.fn();

vi.mock("../../../api/client", () => ({
  apiClient: {
    listConversations: (...args: unknown[]) => mockListConversations(...args),
  },
}));

const { useActiveConversationsByIssue } = await import(
  "../useActiveConversationsByIssue"
);

function makeWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return ({ children }: { children: React.ReactNode }) =>
    React.createElement(QueryClientProvider, { client: queryClient }, children);
}

function makeSummary(
  overrides: Partial<ConversationSummary> &
    Pick<ConversationSummary, "conversation_id">,
): ConversationSummary {
  return {
    title: null,
    agent_name: null,
    status: "active",
    event_count: 0,
    last_event_preview: null,
    creator: "alice",
    spawned_from: null,
    created_at: "2026-05-24T00:00:00Z",
    updated_at: "2026-05-24T00:00:00Z",
    ...overrides,
  };
}

beforeEach(() => {
  mockListConversations.mockReset();
});

describe("useActiveConversationsByIssue", () => {
  it("issues a single batched request for the union of issue ids", async () => {
    mockListConversations.mockResolvedValue({
      conversations: [],
    } satisfies ListConversationsResponse);

    const { result } = renderHook(
      () => useActiveConversationsByIssue(["i-bbb", "i-aaa"]),
      { wrapper: makeWrapper() },
    );

    await waitFor(() => {
      expect(mockListConversations).toHaveBeenCalledTimes(1);
    });
    expect(mockListConversations).toHaveBeenCalledWith({
      spawned_from_ids: "i-aaa,i-bbb",
      include_deleted: false,
    });
    expect(result.current.size).toBe(0);
  });

  it("maps the most-recently-updated non-closed conversation to its issue", async () => {
    const older = makeSummary({
      conversation_id: "c-1",
      spawned_from: "i-aaa",
      status: "idle",
      updated_at: "2026-05-24T00:00:00Z",
    });
    const newer = makeSummary({
      conversation_id: "c-2",
      spawned_from: "i-aaa",
      status: "active",
      updated_at: "2026-05-25T00:00:00Z",
    });
    const otherIssue = makeSummary({
      conversation_id: "c-3",
      spawned_from: "i-bbb",
      status: "active",
      updated_at: "2026-05-24T00:00:00Z",
    });
    const closed = makeSummary({
      conversation_id: "c-4",
      spawned_from: "i-ccc",
      status: "closed",
      updated_at: "2026-05-25T00:00:00Z",
    });

    mockListConversations.mockResolvedValue({
      conversations: [older, newer, otherIssue, closed],
    } satisfies ListConversationsResponse);

    const { result } = renderHook(
      () => useActiveConversationsByIssue(["i-aaa", "i-bbb", "i-ccc"]),
      { wrapper: makeWrapper() },
    );

    await waitFor(() => {
      expect(result.current.size).toBe(2);
    });
    expect(result.current.get("i-aaa")?.conversation_id).toBe("c-2");
    expect(result.current.get("i-bbb")?.conversation_id).toBe("c-3");
    expect(result.current.has("i-ccc")).toBe(false);
  });

  it("does not issue a request when the issue id set is empty", () => {
    renderHook(() => useActiveConversationsByIssue([]), {
      wrapper: makeWrapper(),
    });
    expect(mockListConversations).not.toHaveBeenCalled();
  });
});
