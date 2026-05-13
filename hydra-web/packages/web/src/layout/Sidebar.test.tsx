// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import React from "react";
import { MemoryRouter } from "react-router-dom";

// --- Mocks ---

vi.mock("@hydra/ui", () => ({
  Avatar: ({ name }: { name: string }) => <div data-testid="avatar">{name}</div>,
  Tooltip: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

vi.mock("../features/auth/useAuth", () => ({
  useAuth: () => ({
    user: { actor: { kind: "User", username: "alice" } },
    logout: vi.fn(),
    loading: false,
  }),
}));

vi.mock("../api/auth", () => ({
  actorDisplayName: () => "Alice",
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
  return render(
    <MemoryRouter initialEntries={[initialEntry]}>
      <Sidebar connectionState="connected" />
    </MemoryRouter>,
  );
}

const STORAGE_PREFIX = "hydra:sidebar:section:";

describe("Sidebar section collapse", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  afterEach(() => {
    cleanup();
    window.localStorage.clear();
  });

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
    expect(screen.getByTestId("sidebar-section-issues-more")).toBeTruthy();
    expect(screen.getByTestId("sidebar-section-documents-more")).toBeTruthy();
    expect(screen.getByTestId("sidebar-context-repositories")).toBeTruthy();
    expect(screen.getByTestId("sidebar-context-secrets")).toBeTruthy();
  });

  it("collapses a section when its header is clicked and hides its body", () => {
    renderSidebar();
    const header = screen.getByTestId("sidebar-section-issues");
    fireEvent.click(header);
    expect(header.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByTestId("sidebar-section-issues-more")).toBeNull();
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
  beforeEach(() => {
    window.localStorage.clear();
  });

  afterEach(() => {
    cleanup();
    window.localStorage.clear();
  });

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
  beforeEach(() => {
    window.localStorage.clear();
  });

  afterEach(() => {
    cleanup();
    window.localStorage.clear();
  });

  it("highlights only Issues > More on /?selected=your-issues", () => {
    renderSidebar("/?selected=your-issues");
    const issuesMore = screen.getByTestId("sidebar-section-issues-more");
    const patches = screen.getByTestId("sidebar-patches");
    expect(issuesMore.className).toContain("navItemActive");
    expect(issuesMore.getAttribute("aria-current")).toBe("page");
    expect(patches.className).not.toContain("navItemActive");
    expect(patches.getAttribute("aria-current")).toBeNull();
  });

  it("highlights only Patches on /?selected=patches", () => {
    renderSidebar("/?selected=patches");
    const issuesMore = screen.getByTestId("sidebar-section-issues-more");
    const patches = screen.getByTestId("sidebar-patches");
    expect(patches.className).toContain("navItemActive");
    expect(patches.getAttribute("aria-current")).toBe("page");
    expect(issuesMore.className).not.toContain("navItemActive");
    expect(issuesMore.getAttribute("aria-current")).toBeNull();
  });

  it("highlights neither on / with no selected param", () => {
    renderSidebar("/");
    const issuesMore = screen.getByTestId("sidebar-section-issues-more");
    const patches = screen.getByTestId("sidebar-patches");
    expect(issuesMore.className).not.toContain("navItemActive");
    expect(patches.className).not.toContain("navItemActive");
    expect(issuesMore.getAttribute("aria-current")).toBeNull();
    expect(patches.getAttribute("aria-current")).toBeNull();
  });

  it("highlights neither on a non-/ pathname even with a matching selected param", () => {
    renderSidebar("/documents?selected=patches");
    const issuesMore = screen.getByTestId("sidebar-section-issues-more");
    const patches = screen.getByTestId("sidebar-patches");
    expect(issuesMore.className).not.toContain("navItemActive");
    expect(patches.className).not.toContain("navItemActive");
    expect(issuesMore.getAttribute("aria-current")).toBeNull();
    expect(patches.getAttribute("aria-current")).toBeNull();
  });
});
