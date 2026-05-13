// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import React from "react";
import { MemoryRouter } from "react-router-dom";
import type { ConversationSummary } from "@hydra/api";

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

let mockConversations: ConversationSummary[] | undefined = [];
vi.mock("../features/chat/useConversations", () => ({
  useConversations: () => ({
    data: mockConversations,
    isLoading: false,
    error: null,
  }),
}));

vi.mock("./Sidebar.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("./SidebarDocumentTree", () => ({
  SidebarDocumentTree: () => <div data-testid="sidebar-doc-tree-mock" />,
}));

// --- Import after mocks ---
const { Sidebar } = await import("./Sidebar");

function renderSidebar(
  overrides: {
    hidden?: boolean;
    onHide?: () => void;
    initialEntry?: string;
  } = {},
) {
  return render(
    <MemoryRouter initialEntries={[overrides.initialEntry ?? "/"]}>
      <Sidebar
        connectionState="connected"
        hidden={overrides.hidden ?? false}
        onHide={overrides.onHide ?? (() => {})}
      />
    </MemoryRouter>,
  );
}

const STORAGE_PREFIX = "hydra:sidebar:section:";

function makeConversation(
  overrides: Partial<ConversationSummary> & {
    conversation_id: string;
    updated_at: string;
  },
): ConversationSummary {
  return {
    conversation_id: overrides.conversation_id,
    title: overrides.title ?? null,
    agent_name: overrides.agent_name ?? null,
    status: overrides.status ?? "idle",
    event_count: overrides.event_count ?? 0,
    last_event_preview: overrides.last_event_preview ?? null,
    creator: overrides.creator ?? "alice",
    created_at: overrides.created_at ?? overrides.updated_at,
    updated_at: overrides.updated_at,
  };
}

describe("Sidebar section collapse", () => {
  beforeEach(() => {
    window.localStorage.clear();
    mockConversations = [];
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
    mockConversations = [];
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

describe("Sidebar dashboard active state", () => {
  beforeEach(() => {
    window.localStorage.clear();
    mockConversations = [];
  });

  afterEach(() => {
    cleanup();
    window.localStorage.clear();
  });

  it("highlights only Issues > More on /?selected=your-issues", () => {
    renderSidebar({ initialEntry: "/?selected=your-issues" });
    const issuesMore = screen.getByTestId("sidebar-section-issues-more");
    const patches = screen.getByTestId("sidebar-patches");
    expect(issuesMore.className).toContain("navItemActive");
    expect(issuesMore.getAttribute("aria-current")).toBe("page");
    expect(patches.className).not.toContain("navItemActive");
    expect(patches.getAttribute("aria-current")).toBeNull();
  });

  it("highlights only Patches on /?selected=patches", () => {
    renderSidebar({ initialEntry: "/?selected=patches" });
    const issuesMore = screen.getByTestId("sidebar-section-issues-more");
    const patches = screen.getByTestId("sidebar-patches");
    expect(patches.className).toContain("navItemActive");
    expect(patches.getAttribute("aria-current")).toBe("page");
    expect(issuesMore.className).not.toContain("navItemActive");
    expect(issuesMore.getAttribute("aria-current")).toBeNull();
  });

  it("highlights neither on / with no selected param", () => {
    renderSidebar({ initialEntry: "/" });
    const issuesMore = screen.getByTestId("sidebar-section-issues-more");
    const patches = screen.getByTestId("sidebar-patches");
    expect(issuesMore.className).not.toContain("navItemActive");
    expect(patches.className).not.toContain("navItemActive");
    expect(issuesMore.getAttribute("aria-current")).toBeNull();
    expect(patches.getAttribute("aria-current")).toBeNull();
  });

  it("highlights neither on a non-/ pathname even with a matching selected param", () => {
    renderSidebar({ initialEntry: "/documents?selected=patches" });
    const issuesMore = screen.getByTestId("sidebar-section-issues-more");
    const patches = screen.getByTestId("sidebar-patches");
    expect(issuesMore.className).not.toContain("navItemActive");
    expect(patches.className).not.toContain("navItemActive");
    expect(issuesMore.getAttribute("aria-current")).toBeNull();
    expect(patches.getAttribute("aria-current")).toBeNull();
  });
});

describe("Sidebar Chats section", () => {
  beforeEach(() => {
    window.localStorage.clear();
    mockConversations = [];
  });

  afterEach(() => {
    cleanup();
    window.localStorage.clear();
    mockConversations = [];
  });

  it("renders only the More link when there are no conversations", () => {
    mockConversations = [];
    renderSidebar();
    expect(screen.getByTestId("sidebar-section-chats-more")).toBeTruthy();
    // No chat rows should be present.
    expect(
      document.querySelectorAll('[data-testid^="sidebar-chat-row-"]').length,
    ).toBe(0);
  });

  it("shows fewer rows when there are fewer than three conversations", () => {
    mockConversations = [
      makeConversation({
        conversation_id: "c-only",
        title: "Only",
        updated_at: "2026-05-13T10:00:00Z",
      }),
    ];
    renderSidebar();
    expect(screen.getByTestId("sidebar-chat-row-c-only")).toBeTruthy();
    expect(
      document.querySelectorAll('[data-testid^="sidebar-chat-row-"]').length,
    ).toBe(1);
    expect(screen.getByTestId("sidebar-section-chats-more")).toBeTruthy();
  });

  it("renders the top three conversations sorted by updated_at desc", () => {
    mockConversations = [
      makeConversation({
        conversation_id: "c-old",
        title: "Oldest",
        updated_at: "2026-05-10T10:00:00Z",
      }),
      makeConversation({
        conversation_id: "c-new",
        title: "Newest",
        updated_at: "2026-05-13T18:00:00Z",
      }),
      makeConversation({
        conversation_id: "c-fourth",
        title: "Fourth",
        updated_at: "2026-05-09T08:00:00Z",
      }),
      makeConversation({
        conversation_id: "c-mid",
        title: "Mid",
        updated_at: "2026-05-12T12:00:00Z",
      }),
    ];
    renderSidebar();

    const rows = Array.from(
      document.querySelectorAll<HTMLAnchorElement>(
        '[data-testid^="sidebar-chat-row-"]',
      ),
    );
    expect(rows.map((r) => r.getAttribute("data-testid"))).toEqual([
      "sidebar-chat-row-c-new",
      "sidebar-chat-row-c-mid",
      "sidebar-chat-row-c-old",
    ]);
    expect(rows.map((r) => r.getAttribute("href"))).toEqual([
      "/chat/c-new",
      "/chat/c-mid",
      "/chat/c-old",
    ]);
    expect(rows.map((r) => r.textContent)).toEqual(["Newest", "Mid", "Oldest"]);
    expect(
      screen.getByTestId("sidebar-section-chats-more").getAttribute("href"),
    ).toBe("/chat");
  });

  it("falls back to last_event_preview then 'Untitled conversation' for the row title", () => {
    mockConversations = [
      makeConversation({
        conversation_id: "c-preview",
        title: null,
        last_event_preview: "hello world",
        updated_at: "2026-05-13T12:00:00Z",
      }),
      makeConversation({
        conversation_id: "c-empty",
        title: null,
        last_event_preview: null,
        updated_at: "2026-05-13T11:00:00Z",
      }),
    ];
    renderSidebar();
    expect(
      screen.getByTestId("sidebar-chat-row-c-preview").textContent,
    ).toBe("hello world");
    expect(screen.getByTestId("sidebar-chat-row-c-empty").textContent).toBe(
      "Untitled conversation",
    );
  });
});
