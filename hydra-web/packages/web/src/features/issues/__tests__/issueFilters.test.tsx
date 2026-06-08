// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type { Filter } from "../../filters";

// Spy on each list-X call so we can assert which fired on a given render.
const listPatches = vi.fn(() =>
  Promise.resolve({ patches: [], next_cursor: null, total_count: 0n }),
);
const listSessions = vi.fn(() =>
  Promise.resolve({ sessions: [], next_cursor: null, total_count: 0n }),
);
const listConversations = vi.fn(() => Promise.resolve([]));
const listIssues = vi.fn(() =>
  Promise.resolve({ issues: [], next_cursor: null, total_count: 0n }),
);
const listAgents = vi.fn(() => Promise.resolve({ agents: [] }));
const listUsers = vi.fn(() => Promise.resolve({ users: [] }));

const listProjects = vi.fn(() =>
  Promise.resolve({
    projects: [
      {
        project_id: "j-engv2",
        version: 1,
        project: {
          key: "engineering-v2",
          name: "Engineering v2",
          statuses: [],
          default_status_key: "inbox",
          creator: "alice",
          deleted: false,
        },
      },
    ],
  }),
);
const getProjectStatuses = vi.fn((projectId: string) =>
  Promise.resolve({
    statuses: [
      { key: "inbox", label: "Inbox", color: "#aaa" },
      { key: "backlog", label: "Backlog", color: "#bbb" },
      { key: "pending", label: "Pending", color: "#ccc" },
      { key: "in-development", label: "In dev", color: "#ddd" },
      { key: "in-review", label: "In review", color: "#eee" },
      { key: "pending-release", label: "Pending release", color: "#fff" },
    ],
    default_status_key: "inbox",
    _project_id: projectId, // unused; here to satisfy the call signature
  }),
);
const getDefaultProjectStatuses = vi.fn(() =>
  Promise.resolve({
    statuses: [
      { key: "open", label: "Open", color: "#111" },
      { key: "in-progress", label: "In progress", color: "#222" },
      { key: "failed", label: "Failed", color: "#333" },
      { key: "closed", label: "Closed", color: "#444" },
      { key: "dropped", label: "Dropped", color: "#555" },
    ],
    default_status_key: "open",
  }),
);

vi.mock("../../../api/client", () => ({
  apiClient: {
    listPatches: () => listPatches(),
    listSessions: () => listSessions(),
    listConversations: () => listConversations(),
    listIssues: () => listIssues(),
    listAgents: () => listAgents(),
    listUsers: () => listUsers(),
    listProjects: () => listProjects(),
    getProjectStatuses: (projectId: string) => getProjectStatuses(projectId),
    getDefaultProjectStatuses: () => getDefaultProjectStatuses(),
  },
}));

// The Status filter options pull StatusChip into the test tree once the
// project's status list resolves. Stub it to a span keyed by the status key
// so tests can assert which options the dropdown produced without pulling in
// the StatusChip CSS module.
vi.mock("../../projects/StatusChip", () => ({
  StatusChip: ({
    definition,
    fallbackKey,
  }: {
    definition?: { key: string } | null;
    fallbackKey?: string | null;
  }) => (
    <span data-testid={`status-chip-${definition?.key ?? fallbackKey ?? "empty"}`}>
      {definition?.key ?? fallbackKey ?? ""}
    </span>
  ),
}));

// `useUserOptions` (called by `useIssueFilters`) renders `<Avatar>` chips. Stub
// `@hydra/ui` so the test doesn't pull in CSS Modules / icon resolution.
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

// userOptions.module.css references styles by class name; stub it.
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

const { useIssueFilters } = await import("../issueFilters");

describe("useIssueFilters lazy option lists", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("does NOT fetch any relation-picker list on mount with no relation filter and menu closed", async () => {
    const { result } = renderHook(() => useIssueFilters(), {
      wrapper: makeWrapper(),
    });

    // The hook resolves synchronously; wait a tick to let useQuery effects
    // settle so any wrongly-eager fetch would have registered by now.
    await waitFor(() => {
      expect(result.current).toBeTruthy();
    });

    expect(listPatches).not.toHaveBeenCalled();
    expect(listSessions).not.toHaveBeenCalled();
    expect(listConversations).not.toHaveBeenCalled();
    expect(listIssues).not.toHaveBeenCalled();
  });

  it("fetches all four lists when the add-filter menu opens", async () => {
    const wrapper = makeWrapper();
    const { rerender } = renderHook(
      ({ addMenuOpen }: { addMenuOpen: boolean }) =>
        useIssueFilters({ addMenuOpen }),
      { wrapper, initialProps: { addMenuOpen: false } },
    );

    expect(listPatches).not.toHaveBeenCalled();
    expect(listSessions).not.toHaveBeenCalled();
    expect(listConversations).not.toHaveBeenCalled();
    expect(listIssues).not.toHaveBeenCalled();

    rerender({ addMenuOpen: true });

    await waitFor(() => {
      expect(listPatches).toHaveBeenCalledTimes(1);
      expect(listSessions).toHaveBeenCalledTimes(1);
      expect(listConversations).toHaveBeenCalledTimes(1);
      expect(listIssues).toHaveBeenCalledTimes(1);
    });
  });

  it("fetches ONLY the patches list when only a relatedPatch chip is present", async () => {
    renderHook(
      () => useIssueFilters({ filters: [chip("relatedPatch", "p-aa")] }),
      { wrapper: makeWrapper() },
    );

    await waitFor(() => {
      expect(listPatches).toHaveBeenCalledTimes(1);
    });

    expect(listSessions).not.toHaveBeenCalled();
    expect(listConversations).not.toHaveBeenCalled();
    expect(listIssues).not.toHaveBeenCalled();
  });

  it("fetches sessions for relatedSession, conversations for relatedChat, issues for parentOrChild", async () => {
    renderHook(
      () =>
        useIssueFilters({
          filters: [
            chip("relatedSession", "s-aa"),
            chip("relatedChat", "c-aa"),
            chip("parentOrChild", "i-aa"),
          ],
        }),
      { wrapper: makeWrapper() },
    );

    await waitFor(() => {
      expect(listSessions).toHaveBeenCalledTimes(1);
      expect(listConversations).toHaveBeenCalledTimes(1);
      expect(listIssues).toHaveBeenCalledTimes(1);
    });

    expect(listPatches).not.toHaveBeenCalled();
  });

  it("a non-relation chip (status) does not enable any relation list", async () => {
    const { result } = renderHook(
      () => useIssueFilters({ filters: [chip("status", "open")] }),
      { wrapper: makeWrapper() },
    );

    await waitFor(() => {
      expect(result.current).toBeTruthy();
    });

    expect(listPatches).not.toHaveBeenCalled();
    expect(listSessions).not.toHaveBeenCalled();
    expect(listConversations).not.toHaveBeenCalled();
    expect(listIssues).not.toHaveBeenCalled();
  });
});

describe("useIssueFilters project + dynamic status options", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("exposes a `project` filter dimension with options from useProjects()", async () => {
    const { result } = renderHook(() => useIssueFilters({ addMenuOpen: true }), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.project).toBeDefined();
      expect(result.current.project.options.length).toBeGreaterThan(0);
    });

    expect(result.current.project.options[0]).toMatchObject({
      value: "j-engv2",
      label: "engineering-v2",
    });
  });

  it("derives the Status filter options from the default project when no project chip is set", async () => {
    const { result } = renderHook(() => useIssueFilters(), {
      wrapper: makeWrapper(),
    });

    await waitFor(() => {
      expect(result.current.status.options.length).toBeGreaterThan(0);
    });

    const values = result.current.status.options.map((o) => o.value);
    expect(values).toEqual(["open", "in-progress", "failed", "closed", "dropped"]);
    expect(getDefaultProjectStatuses).toHaveBeenCalled();
  });

  it("re-scopes the Status filter options to the selected project when a project chip is active", async () => {
    const { result } = renderHook(
      () => useIssueFilters({ filters: [chip("project", "j-engv2")] }),
      { wrapper: makeWrapper() },
    );

    await waitFor(() => {
      const values = result.current.status.options.map((o) => o.value);
      expect(values).toEqual([
        "inbox",
        "backlog",
        "pending",
        "in-development",
        "in-review",
        "pending-release",
      ]);
    });

    expect(getProjectStatuses).toHaveBeenCalledWith("j-engv2");
    // None of the legacy 5-enum strings appears when the project filter is set
    // to engineering-v2 — that's the Bundle A assertion from
    // tests/e2e/scenarios/per-project-status-pipeline.md.
    const values = result.current.status.options.map((o) => o.value);
    expect(values).not.toContain("open");
    expect(values).not.toContain("in-progress");
    expect(values).not.toContain("failed");
    expect(values).not.toContain("closed");
    expect(values).not.toContain("dropped");
  });
});
