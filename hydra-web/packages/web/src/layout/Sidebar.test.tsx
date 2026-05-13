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

// --- Import after mocks ---
const { Sidebar } = await import("./Sidebar");

function renderSidebar(
  overrides: { hidden?: boolean; onHide?: () => void } = {},
) {
  return render(
    <MemoryRouter>
      <Sidebar
        connectionState="connected"
        hidden={overrides.hidden ?? false}
        onHide={overrides.onHide ?? (() => {})}
      />
    </MemoryRouter>,
  );
}

const STORAGE_PREFIX = "hydra.sidebar.section.";

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

  it("renders sessions/search header slots as no-op buttons", () => {
    renderSidebar();
    expect(screen.getByTestId("sidebar-header-sessions").tagName).toBe("BUTTON");
    expect(screen.getByTestId("sidebar-header-search").tagName).toBe("BUTTON");
    // Clicking them should not crash.
    fireEvent.click(screen.getByTestId("sidebar-header-sessions"));
    fireEvent.click(screen.getByTestId("sidebar-header-search"));
  });

  it("invokes onHide when the hide button is clicked", () => {
    const onHide = vi.fn();
    renderSidebar({ onHide });
    fireEvent.click(screen.getByTestId("sidebar-header-hide"));
    expect(onHide).toHaveBeenCalledTimes(1);
  });

  it("marks the sidebar as inert/aria-hidden when hidden is true", () => {
    renderSidebar({ hidden: true });
    const nav = screen.getByTestId("sidebar");
    expect(nav.getAttribute("aria-hidden")).toBe("true");
    expect(nav.hasAttribute("inert")).toBe(true);
  });

  it("does not mark the sidebar inert when hidden is false", () => {
    renderSidebar({ hidden: false });
    const nav = screen.getByTestId("sidebar");
    expect(nav.getAttribute("aria-hidden")).toBeNull();
    expect(nav.hasAttribute("inert")).toBe(false);
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
