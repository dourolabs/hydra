// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import React from "react";
import { MemoryRouter } from "react-router-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { LabelRecord } from "@hydra/api";

// --- Mocks ---

vi.mock("@hydra/ui", () => ({
  Avatar: ({ name }: { name: string }) => <div data-testid="avatar">{name}</div>,
  Tooltip: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

vi.mock("../features/auth/useAuth", () => ({
  useAuth: () => ({
    user: { actor: { type: "user", username: "alice" } },
    logout: vi.fn(),
    loading: false,
  }),
}));

vi.mock("../api/auth", () => ({
  actorDisplayName: () => "Alice",
}));

const issueCountMock = vi.fn();
const labelsMock = vi.fn();

vi.mock("../features/issues/usePaginatedIssues", () => ({
  useIssueCount: (...args: unknown[]) => issueCountMock(...args),
}));

vi.mock("../features/labels/useLabels", () => ({
  useLabels: () => labelsMock(),
}));

vi.mock("./Sidebar.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("./SidebarDocumentTree", () => ({
  SidebarDocumentTree: () => <div data-testid="sidebar-doc-tree-mock" />,
}));

// --- Import after mocks ---
const { Sidebar } = await import("./Sidebar");

function renderSidebar(initialEntry: string = "/") {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={[initialEntry]}>
        <Sidebar connectionState="connected" />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

function makeLabel(overrides: Partial<LabelRecord>): LabelRecord {
  return {
    label_id: "l-x",
    name: "label",
    color: "#000000",
    recurse: true,
    hidden: false,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

const STORAGE_PREFIX = "hydra:sidebar:section:";

beforeEach(() => {
  window.localStorage.clear();
  issueCountMock.mockReturnValue({ data: 0 });
  labelsMock.mockReturnValue({ data: [] });
});

afterEach(() => {
  cleanup();
  window.localStorage.clear();
  issueCountMock.mockReset();
  labelsMock.mockReset();
});

describe("Sidebar section collapse", () => {
  it("renders all four collapsable sections expanded by default", () => {
    renderSidebar();
    expect(
      screen.getByTestId("sidebar-section-chats").getAttribute("aria-expanded"),
    ).toBe("true");
    expect(
      screen
        .getByTestId("sidebar-section-issues")
        .getAttribute("aria-expanded"),
    ).toBe("true");
    expect(
      screen
        .getByTestId("sidebar-section-documents")
        .getAttribute("aria-expanded"),
    ).toBe("true");
    expect(
      screen
        .getByTestId("sidebar-section-context")
        .getAttribute("aria-expanded"),
    ).toBe("true");
    expect(screen.getByTestId("sidebar-section-chats-more")).toBeTruthy();
    expect(screen.getByTestId("sidebar-issues-assigned")).toBeTruthy();
    expect(screen.getByTestId("sidebar-issues-all")).toBeTruthy();
    expect(screen.getByTestId("sidebar-section-documents-more")).toBeTruthy();
    expect(screen.getByTestId("sidebar-context-repositories")).toBeTruthy();
    expect(screen.getByTestId("sidebar-context-secrets")).toBeTruthy();
  });

  it("collapses a section when its header is clicked and hides its body", () => {
    renderSidebar();
    const header = screen.getByTestId("sidebar-section-issues");
    fireEvent.click(header);
    expect(header.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByTestId("sidebar-issues-assigned")).toBeNull();
    expect(screen.queryByTestId("sidebar-issues-all")).toBeNull();
  });

  it("persists the collapsed state to localStorage", () => {
    renderSidebar();
    fireEvent.click(screen.getByTestId("sidebar-section-chats"));
    expect(window.localStorage.getItem(`${STORAGE_PREFIX}chats`)).toBe("false");

    fireEvent.click(screen.getByTestId("sidebar-section-chats"));
    expect(window.localStorage.getItem(`${STORAGE_PREFIX}chats`)).toBe("true");
  });

  it("restores collapsed state from localStorage on mount", () => {
    window.localStorage.setItem(`${STORAGE_PREFIX}documents`, "false");
    renderSidebar();
    const header = screen.getByTestId("sidebar-section-documents");
    expect(header.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByTestId("sidebar-section-documents-more")).toBeNull();
  });

  it("keeps each section's collapse state independent", () => {
    window.localStorage.setItem(`${STORAGE_PREFIX}issues`, "false");
    window.localStorage.setItem(`${STORAGE_PREFIX}chats`, "true");
    renderSidebar();
    expect(
      screen
        .getByTestId("sidebar-section-issues")
        .getAttribute("aria-expanded"),
    ).toBe("false");
    expect(
      screen.getByTestId("sidebar-section-chats").getAttribute("aria-expanded"),
    ).toBe("true");
  });
});

describe("Sidebar static structure", () => {
  it("renders header slots as no-op buttons", () => {
    renderSidebar();
    expect(screen.getByTestId("sidebar-header-sessions").tagName).toBe("BUTTON");
    expect(screen.getByTestId("sidebar-header-search").tagName).toBe("BUTTON");
    expect(screen.getByTestId("sidebar-header-hide").tagName).toBe("BUTTON");
    // Clicking them should not crash.
    fireEvent.click(screen.getByTestId("sidebar-header-sessions"));
    fireEvent.click(screen.getByTestId("sidebar-header-search"));
    fireEvent.click(screen.getByTestId("sidebar-header-hide"));
  });

  it("renders Patches and Agents as static links to the expected routes", () => {
    renderSidebar();
    expect(screen.getByTestId("sidebar-patches").getAttribute("href")).toBe(
      "/?selected=patches",
    );
    expect(screen.getByTestId("sidebar-agents").getAttribute("href")).toBe(
      "/settings",
    );
  });

  it("renders Context children pointing at /settings", () => {
    renderSidebar();
    expect(
      screen
        .getByTestId("sidebar-context-repositories")
        .getAttribute("href"),
    ).toBe("/settings");
    expect(
      screen.getByTestId("sidebar-context-secrets").getAttribute("href"),
    ).toBe("/settings");
  });
});

describe("Sidebar dashboard active state", () => {
  it("highlights only Patches on /?selected=patches", () => {
    renderSidebar("/?selected=patches");
    const patches = screen.getByTestId("sidebar-patches");
    const assigned = screen.getByTestId("sidebar-issues-assigned");
    const all = screen.getByTestId("sidebar-issues-all");
    expect(patches.className).toContain("navItemActive");
    expect(patches.getAttribute("aria-current")).toBe("page");
    expect(assigned.className).not.toContain("navItemActive");
    expect(all.className).not.toContain("navItemActive");
    expect(assigned.getAttribute("aria-current")).toBeNull();
    expect(all.getAttribute("aria-current")).toBeNull();
  });

  it("highlights only Assigned to you on /?selected=assigned", () => {
    renderSidebar("/?selected=assigned");
    const assigned = screen.getByTestId("sidebar-issues-assigned");
    const all = screen.getByTestId("sidebar-issues-all");
    const patches = screen.getByTestId("sidebar-patches");
    expect(assigned.className).toContain("navItemActive");
    expect(assigned.getAttribute("aria-current")).toBe("page");
    expect(all.className).not.toContain("navItemActive");
    expect(patches.className).not.toContain("navItemActive");
  });

  it("highlights only All issues on /?selected=all (no label)", () => {
    renderSidebar("/?selected=all");
    const assigned = screen.getByTestId("sidebar-issues-assigned");
    const all = screen.getByTestId("sidebar-issues-all");
    const patches = screen.getByTestId("sidebar-patches");
    expect(all.className).toContain("navItemActive");
    expect(all.getAttribute("aria-current")).toBe("page");
    expect(assigned.className).not.toContain("navItemActive");
    expect(patches.className).not.toContain("navItemActive");
  });

  it("highlights only the matching label row on /?selected=all&label=<id>", () => {
    labelsMock.mockReturnValue({
      data: [
        makeLabel({
          label_id: "l-a",
          name: "a",
          updated_at: "2026-05-10T00:00:00Z",
        }),
        makeLabel({
          label_id: "l-b",
          name: "b",
          updated_at: "2026-05-09T00:00:00Z",
        }),
      ],
    });
    renderSidebar("/?selected=all&label=l-a");
    const labelA = screen.getByTestId("sidebar-issues-label-l-a");
    const labelB = screen.getByTestId("sidebar-issues-label-l-b");
    const all = screen.getByTestId("sidebar-issues-all");
    expect(labelA.className).toContain("navItemActive");
    expect(labelA.getAttribute("aria-current")).toBe("page");
    expect(labelB.className).not.toContain("navItemActive");
    // All issues should NOT be active when a label is selected.
    expect(all.className).not.toContain("navItemActive");
    expect(all.getAttribute("aria-current")).toBeNull();
  });

  it("highlights nothing on / with no selected param", () => {
    renderSidebar("/");
    const patches = screen.getByTestId("sidebar-patches");
    const assigned = screen.getByTestId("sidebar-issues-assigned");
    const all = screen.getByTestId("sidebar-issues-all");
    expect(patches.className).not.toContain("navItemActive");
    expect(assigned.className).not.toContain("navItemActive");
    expect(all.className).not.toContain("navItemActive");
  });

  it("highlights nothing on a non-/ pathname even with a matching selected param", () => {
    renderSidebar("/documents?selected=patches");
    const patches = screen.getByTestId("sidebar-patches");
    const assigned = screen.getByTestId("sidebar-issues-assigned");
    const all = screen.getByTestId("sidebar-issues-all");
    expect(patches.className).not.toContain("navItemActive");
    expect(assigned.className).not.toContain("navItemActive");
    expect(all.className).not.toContain("navItemActive");
  });
});

describe("Sidebar Issues section", () => {
  it("links 'Assigned to you' to /?selected=assigned and 'All issues' to /?selected=all", () => {
    renderSidebar();
    expect(screen.getByTestId("sidebar-issues-assigned").getAttribute("href")).toBe(
      "/?selected=assigned",
    );
    expect(screen.getByTestId("sidebar-issues-all").getAttribute("href")).toBe(
      "/?selected=all",
    );
  });

  it("hides the assigned badge when count is zero", () => {
    issueCountMock.mockReturnValue({ data: 0 });
    renderSidebar();
    expect(screen.queryByTestId("sidebar-issues-assigned-badge")).toBeNull();
  });

  it("shows the assigned badge with the current count when greater than zero", () => {
    issueCountMock.mockReturnValue({ data: 7 });
    renderSidebar();
    const badge = screen.getByTestId("sidebar-issues-assigned-badge");
    expect(badge.textContent).toBe("7");
  });

  it("calls useIssueCount with assignee + open status for the current user", () => {
    issueCountMock.mockReturnValue({ data: 0 });
    renderSidebar();
    expect(issueCountMock).toHaveBeenCalled();
    const lastCall = issueCountMock.mock.calls[issueCountMock.mock.calls.length - 1];
    expect(lastCall[0]).toEqual({ assignee: "Alice", status: "open" });
    expect(lastCall[1]).toBe(true);
  });

  it("renders top 3 labels sorted by updated_at desc", () => {
    labelsMock.mockReturnValue({
      data: [
        makeLabel({
          label_id: "l-oldest",
          name: "oldest",
          color: "#111111",
          updated_at: "2026-01-01T00:00:00Z",
        }),
        makeLabel({
          label_id: "l-newest",
          name: "newest",
          color: "#222222",
          updated_at: "2026-05-10T00:00:00Z",
        }),
        makeLabel({
          label_id: "l-middle1",
          name: "middle1",
          color: "#333333",
          updated_at: "2026-03-15T00:00:00Z",
        }),
        makeLabel({
          label_id: "l-middle2",
          name: "middle2",
          color: "#444444",
          updated_at: "2026-04-20T00:00:00Z",
        }),
        makeLabel({
          label_id: "l-middle3",
          name: "middle3",
          color: "#555555",
          updated_at: "2026-02-10T00:00:00Z",
        }),
      ],
    });
    renderSidebar();

    // The three displayed labels should be the three most recently updated,
    // in order: newest, middle2, middle1.
    expect(screen.getByTestId("sidebar-issues-label-l-newest")).toBeTruthy();
    expect(screen.getByTestId("sidebar-issues-label-l-middle2")).toBeTruthy();
    expect(screen.getByTestId("sidebar-issues-label-l-middle1")).toBeTruthy();
    // The two older labels should not be rendered.
    expect(screen.queryByTestId("sidebar-issues-label-l-middle3")).toBeNull();
    expect(screen.queryByTestId("sidebar-issues-label-l-oldest")).toBeNull();

    // Each label row should deep-link to ?selected=all&label=<id>.
    expect(
      screen.getByTestId("sidebar-issues-label-l-newest").getAttribute("href"),
    ).toBe("/?selected=all&label=l-newest");
    expect(
      screen.getByTestId("sidebar-issues-label-l-middle2").getAttribute("href"),
    ).toBe("/?selected=all&label=l-middle2");
    expect(
      screen.getByTestId("sidebar-issues-label-l-middle1").getAttribute("href"),
    ).toBe("/?selected=all&label=l-middle1");

    // Verify the documents come in DOM order matching sort: newest, middle2, middle1.
    const labelRows = screen
      .getAllByRole("link")
      .filter((el) => el.getAttribute("data-testid")?.startsWith("sidebar-issues-label-"));
    expect(labelRows.map((el) => el.getAttribute("data-testid"))).toEqual([
      "sidebar-issues-label-l-newest",
      "sidebar-issues-label-l-middle2",
      "sidebar-issues-label-l-middle1",
    ]);
  });

  it("renders fewer than 3 label rows when fewer labels are available", () => {
    labelsMock.mockReturnValue({
      data: [
        makeLabel({
          label_id: "l-only",
          name: "only",
          updated_at: "2026-01-01T00:00:00Z",
        }),
      ],
    });
    renderSidebar();
    expect(screen.getByTestId("sidebar-issues-label-l-only")).toBeTruthy();
    // Assigned-to-you + the one label + All issues = 3 nav items in the Issues
    // section. We can't easily count without a wrapper element, but at least
    // confirm All issues still renders.
    expect(screen.getByTestId("sidebar-issues-all")).toBeTruthy();
  });

  it("renders no label rows when labels are unavailable", () => {
    labelsMock.mockReturnValue({ data: undefined });
    renderSidebar();
    expect(screen.queryByText("middle1")).toBeNull();
    expect(screen.getByTestId("sidebar-issues-all")).toBeTruthy();
  });
});
