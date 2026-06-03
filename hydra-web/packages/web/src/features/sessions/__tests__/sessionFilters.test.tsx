// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type { Filter } from "../../filters";

// Spy on each list-X call so we can assert which fired on a given render.
const listIssues = vi.fn(() =>
  Promise.resolve({ issues: [], next_cursor: null, total_count: 0n }),
);
const listPatches = vi.fn(() =>
  Promise.resolve({ patches: [], next_cursor: null, total_count: 0n }),
);
const listConversations = vi.fn(() => Promise.resolve([]));
const listAgents = vi.fn(() => Promise.resolve({ agents: [] }));
const listUsers = vi.fn(() => Promise.resolve({ users: [] }));

vi.mock("../../../api/client", () => ({
  apiClient: {
    listIssues: () => listIssues(),
    listPatches: () => listPatches(),
    listConversations: () => listConversations(),
    listAgents: () => listAgents(),
    listUsers: () => listUsers(),
  },
}));

// `useUserOptions` (called by `useSessionFilters`) renders `<Avatar>` chips.
// Stub `@hydra/ui` so the test doesn't pull in CSS Modules / icon resolution.
vi.mock("@hydra/ui", () => ({
  Avatar: ({ name }: { name: string }) => <span data-testid="avatar">{name}</span>,
  Badge: ({ status }: { status: string }) => <span data-testid="badge">{status}</span>,
  TypeChip: ({ type }: { type: string }) => <span data-testid="type-chip">{type}</span>,
  Icons: new Proxy(
    {},
    {
      get: () => () => <span data-testid="icon" />,
    },
  ),
}));

// CSS Module imports referenced indirectly through options helpers.
vi.mock("../../filters/options/userOptions.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));
vi.mock("../../filters/options/relationOptions.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

function makeWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return ({ children }: { children: React.ReactNode }) =>
    React.createElement(QueryClientProvider, { client: queryClient }, children);
}

function chip(id: string, value: string): Filter {
  return { _uid: `u-${id}-${value}`, id, op: "in", values: [value] };
}

const { useSessionFilters } = await import("../sessionFilters");

describe("useSessionFilters lazy option lists", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("does NOT fetch any relation-picker list on mount with no relation filter and menu closed", async () => {
    const { result } = renderHook(() => useSessionFilters(), {
      wrapper: makeWrapper(),
    });

    // The hook resolves synchronously; wait a tick to let useQuery effects
    // settle so any wrongly-eager fetch would have registered by now.
    await waitFor(() => {
      expect(result.current).toBeTruthy();
    });

    expect(listIssues).not.toHaveBeenCalled();
    expect(listPatches).not.toHaveBeenCalled();
    expect(listConversations).not.toHaveBeenCalled();
  });

  it("fetches all three lists when the add-filter menu opens", async () => {
    const wrapper = makeWrapper();
    const { rerender } = renderHook(
      ({ addMenuOpen }: { addMenuOpen: boolean }) =>
        useSessionFilters({ addMenuOpen }),
      { wrapper, initialProps: { addMenuOpen: false } },
    );

    expect(listIssues).not.toHaveBeenCalled();
    expect(listPatches).not.toHaveBeenCalled();
    expect(listConversations).not.toHaveBeenCalled();

    rerender({ addMenuOpen: true });

    await waitFor(() => {
      expect(listIssues).toHaveBeenCalledTimes(1);
      expect(listPatches).toHaveBeenCalledTimes(1);
      expect(listConversations).toHaveBeenCalledTimes(1);
    });
  });

  it("fetches issues for relatedIssue, patches for relatedPatch, conversations for relatedChat", async () => {
    renderHook(
      () =>
        useSessionFilters({
          filters: [
            chip("relatedIssue", "i-aa"),
            chip("relatedPatch", "p-aa"),
            chip("relatedChat", "c-aa"),
          ],
        }),
      { wrapper: makeWrapper() },
    );

    await waitFor(() => {
      expect(listIssues).toHaveBeenCalledTimes(1);
      expect(listPatches).toHaveBeenCalledTimes(1);
      expect(listConversations).toHaveBeenCalledTimes(1);
    });
  });

  it("fetches ONLY the patches list when only a relatedPatch chip is present", async () => {
    renderHook(
      () => useSessionFilters({ filters: [chip("relatedPatch", "p-aa")] }),
      { wrapper: makeWrapper() },
    );

    await waitFor(() => {
      expect(listPatches).toHaveBeenCalledTimes(1);
    });

    expect(listIssues).not.toHaveBeenCalled();
    expect(listConversations).not.toHaveBeenCalled();
  });

  it("a non-relation chip (status) does not enable any relation list", async () => {
    const { result } = renderHook(
      () => useSessionFilters({ filters: [chip("status", "running")] }),
      { wrapper: makeWrapper() },
    );

    await waitFor(() => {
      expect(result.current).toBeTruthy();
    });

    expect(listIssues).not.toHaveBeenCalled();
    expect(listPatches).not.toHaveBeenCalled();
    expect(listConversations).not.toHaveBeenCalled();
  });
});
