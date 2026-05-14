// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup, waitFor } from "@testing-library/react";
import React from "react";
import { MemoryRouter, useLocation } from "react-router-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ConversationSummary, LabelRecord } from "@hydra/api";

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

let mockConversations: ConversationSummary[] | undefined = [];
vi.mock("../features/chat/useConversations", () => ({
  useConversations: () => ({
    data: mockConversations,
    isLoading: false,
    error: null,
  }),
}));

const issueCountMock = vi.fn();
const labelsMock = vi.fn();

vi.mock("../features/issues/usePaginatedIssues", () => ({
  useIssueCount: (...args: unknown[]) => issueCountMock(...args),
}));

vi.mock("../features/labels/useLabels", () => ({
  useLabels: () => labelsMock(),
}));

const getVersionMock = vi.fn();
const createConversationMock = vi.fn();
vi.mock("../api/client", () => ({
  apiClient: {
    getVersion: (...args: unknown[]) => getVersionMock(...args),
    createConversation: (...args: unknown[]) => createConversationMock(...args),
  },
}));

vi.mock("./Sidebar.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("./SidebarDocumentTree", () => ({
  SidebarDocumentTree: () => <div data-testid="sidebar-doc-tree-mock" />,
}));

// --- Import after mocks ---
const { Sidebar } = await import("./Sidebar");

function LocationDisplay() {
  const location = useLocation();
  return (
    <div data-testid="location">
      {location.pathname}
      {location.search}
    </div>
  );
}

function renderSidebar(
  overrides: {
    hidden?: boolean;
    initialEntry?: string;
    onHide?: () => void;
  } = {},
) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={[overrides.initialEntry ?? "/"]}>
        <Sidebar
          connectionState="connected"
          hidden={overrides.hidden ?? false}
          onHide={overrides.onHide ?? (() => {})}
        />
        <LocationDisplay />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

type ChangeListener = (e: MediaQueryListEvent) => void;
function mockMatchMedia(matches: boolean) {
  const listeners: ChangeListener[] = [];
  const mql = {
    matches,
    media: "",
    onchange: null,
    addEventListener: (event: string, handler: ChangeListener) => {
      if (event === "change") listeners.push(handler);
    },
    removeEventListener: (event: string, handler: ChangeListener) => {
      if (event === "change") {
        const idx = listeners.indexOf(handler);
        if (idx !== -1) listeners.splice(idx, 1);
      }
    },
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => true,
  };
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    writable: true,
    value: () => mql as unknown as MediaQueryList,
  });
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

beforeEach(() => {
  window.localStorage.clear();
  mockConversations = [];
  issueCountMock.mockReturnValue({ data: 0 });
  labelsMock.mockReturnValue({ data: [] });
  getVersionMock.mockResolvedValue({ version: "1.2.3" });
  createConversationMock.mockReset();
  // Default matchMedia stub: desktop (no media query matches). Individual
  // tests can override via mockMatchMedia(true) to simulate mobile.
  mockMatchMedia(false);
});

afterEach(() => {
  cleanup();
  window.localStorage.clear();
  mockConversations = [];
  issueCountMock.mockReset();
  labelsMock.mockReset();
  getVersionMock.mockReset();
  createConversationMock.mockReset();
  vi.restoreAllMocks();
});

describe("Sidebar section icons", () => {
  // Each section header renders a chevron SVG plus a section icon SVG, so
  // we expect at least two <svg> children inside the header button.
  const sectionIds = ["chats", "issues", "documents", "context"] as const;

  it.each(sectionIds)(
    "renders an icon SVG inside the %s section header",
    (id) => {
      renderSidebar();
      const header = screen.getByTestId(`sidebar-section-${id}`);
      const svgs = header.querySelectorAll("svg");
      // chevron + icon
      expect(svgs.length).toBeGreaterThanOrEqual(2);
    },
  );

  it("renders an icon SVG inside the Patches standalone link", () => {
    renderSidebar();
    const link = screen.getByTestId("sidebar-patches");
    expect(link.querySelector("svg")).not.toBeNull();
    expect(link.textContent).toContain("Patches");
  });

  it("renders an icon SVG inside the Agents standalone link", () => {
    renderSidebar();
    const link = screen.getByTestId("sidebar-agents");
    expect(link.querySelector("svg")).not.toBeNull();
    expect(link.textContent).toContain("Agents");
  });
});

describe("Sidebar header block removed", () => {
  it("does not render the in-sidebar active-sessions / search / hide buttons", () => {
    renderSidebar();
    expect(screen.queryByTestId("sidebar-header-sessions")).toBeNull();
    expect(screen.queryByTestId("sidebar-header-search")).toBeNull();
    expect(screen.queryByTestId("sidebar-header-hide")).toBeNull();
  });
});

describe("Sidebar hide button", () => {
  it("renders the sidebar-hide button on desktop", () => {
    mockMatchMedia(false);
    renderSidebar();
    expect(screen.getByTestId("sidebar-hide")).toBeTruthy();
  });

  it("calls onHide when the sidebar-hide button is clicked on desktop", () => {
    mockMatchMedia(false);
    const onHide = vi.fn();
    renderSidebar({ onHide });
    fireEvent.click(screen.getByTestId("sidebar-hide"));
    expect(onHide).toHaveBeenCalledTimes(1);
  });

  it("uses 'Hide sidebar' as the aria-label", () => {
    mockMatchMedia(false);
    renderSidebar();
    expect(screen.getByTestId("sidebar-hide").getAttribute("aria-label")).toBe(
      "Hide sidebar",
    );
  });

  it("does NOT render the sidebar-hide button on mobile", () => {
    mockMatchMedia(true);
    renderSidebar();
    expect(screen.queryByTestId("sidebar-hide")).toBeNull();
  });
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
      "/agents",
    );
  });

  it("renders Context children pointing at their dedicated pages", () => {
    renderSidebar();
    expect(
      screen
        .getByTestId("sidebar-context-repositories")
        .getAttribute("href"),
    ).toBe("/repositories");
    expect(
      screen.getByTestId("sidebar-context-secrets").getAttribute("href"),
    ).toBe("/secrets");
  });
});

describe("Sidebar dashboard active state", () => {
  it("highlights only Patches on /?selected=patches", () => {
    renderSidebar({ initialEntry: "/?selected=patches" });
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
    renderSidebar({ initialEntry: "/?selected=assigned" });
    const assigned = screen.getByTestId("sidebar-issues-assigned");
    const all = screen.getByTestId("sidebar-issues-all");
    const patches = screen.getByTestId("sidebar-patches");
    expect(assigned.className).toContain("navItemActive");
    expect(assigned.getAttribute("aria-current")).toBe("page");
    expect(all.className).not.toContain("navItemActive");
    expect(patches.className).not.toContain("navItemActive");
  });

  it("highlights only All issues on /?selected=all (no label)", () => {
    renderSidebar({ initialEntry: "/?selected=all" });
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
    renderSidebar({ initialEntry: "/?selected=all&label=l-a" });
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
    renderSidebar({ initialEntry: "/" });
    const patches = screen.getByTestId("sidebar-patches");
    const assigned = screen.getByTestId("sidebar-issues-assigned");
    const all = screen.getByTestId("sidebar-issues-all");
    expect(patches.className).not.toContain("navItemActive");
    expect(assigned.className).not.toContain("navItemActive");
    expect(all.className).not.toContain("navItemActive");
  });

  it("highlights nothing on a non-/ pathname even with a matching selected param", () => {
    renderSidebar({ initialEntry: "/documents?selected=patches" });
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

describe("Sidebar Chats section", () => {
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

describe("Sidebar + New Chat button", () => {
  it("renders the + New Chat button as the first row in the Chats section", () => {
    renderSidebar();
    const newChatButton = screen.getByTestId("sidebar-chat-new");
    expect(newChatButton).toBeTruthy();
    expect(newChatButton.textContent).toContain("+ New Chat");
  });

  it("calls apiClient.createConversation and navigates to the new conversation on click", async () => {
    createConversationMock.mockResolvedValue({ conversation_id: "c-new42" });
    renderSidebar();
    const newChatButton = screen.getByTestId("sidebar-chat-new");
    fireEvent.click(newChatButton);
    await waitFor(() => {
      expect(createConversationMock).toHaveBeenCalledWith({});
    });
    await waitFor(() => {
      expect(screen.getByTestId("location").textContent).toBe("/chat/c-new42");
    });
  });

  it("disables the button and shows Creating… while the mutation is pending", async () => {
    createConversationMock.mockReturnValue(new Promise(() => {}));
    renderSidebar();
    const newChatButton = screen.getByTestId(
      "sidebar-chat-new",
    ) as HTMLButtonElement;
    fireEvent.click(newChatButton);
    await waitFor(() => {
      expect(newChatButton.textContent).toContain("Creating");
    });
    expect(newChatButton.disabled).toBe(true);
  });
});

describe("Sidebar + New Issue button", () => {
  it("renders the + New Issue button as the first row in the Issues section", () => {
    renderSidebar();
    const newIssueButton = screen.getByTestId("sidebar-issues-new");
    expect(newIssueButton).toBeTruthy();
    expect(newIssueButton.textContent).toContain("+ New Issue");
  });

  it("navigates to /?create-issue=1 on click from a non-dashboard page", () => {
    renderSidebar({ initialEntry: "/documents/foo" });
    fireEvent.click(screen.getByTestId("sidebar-issues-new"));
    expect(screen.getByTestId("location").textContent).toBe(
      "/?create-issue=1",
    );
  });

  it("navigates to /?create-issue=1 on click from the dashboard", () => {
    renderSidebar({ initialEntry: "/" });
    fireEvent.click(screen.getByTestId("sidebar-issues-new"));
    expect(screen.getByTestId("location").textContent).toBe(
      "/?create-issue=1",
    );
  });
});

describe("Sidebar See All active highlight", () => {
  it("highlights Chats See All only when path is exactly /chat", () => {
    renderSidebar({ initialEntry: "/chat" });
    const seeAll = screen.getByTestId("sidebar-section-chats-more");
    expect(seeAll.className).toContain("navItemActive");
    expect(seeAll.textContent).toContain("See All");
  });

  it("does NOT highlight Chats See All on /chat/:id", () => {
    renderSidebar({ initialEntry: "/chat/c-abc" });
    const seeAll = screen.getByTestId("sidebar-section-chats-more");
    expect(seeAll.className).not.toContain("navItemActive");
  });

  it("highlights Documents See All only when path is exactly /documents", () => {
    renderSidebar({ initialEntry: "/documents" });
    const seeAll = screen.getByTestId("sidebar-section-documents-more");
    expect(seeAll.className).toContain("navItemActive");
    expect(seeAll.textContent).toContain("See All");
  });

  it("does NOT highlight Documents See All on /documents/:id", () => {
    renderSidebar({ initialEntry: "/documents/d-abc" });
    const seeAll = screen.getByTestId("sidebar-section-documents-more");
    expect(seeAll.className).not.toContain("navItemActive");
  });
});

describe("Sidebar mobile drawer behaviour", () => {
  it("renders the backdrop only when on mobile and the sidebar is visible", () => {
    mockMatchMedia(true);
    renderSidebar({ hidden: false });
    expect(screen.getByTestId("sidebar-backdrop")).toBeTruthy();
  });

  it("does not render the backdrop on desktop", () => {
    mockMatchMedia(false);
    renderSidebar({ hidden: false });
    expect(screen.queryByTestId("sidebar-backdrop")).toBeNull();
  });

  it("does not render the backdrop on mobile when the sidebar is hidden", () => {
    mockMatchMedia(true);
    renderSidebar({ hidden: true });
    expect(screen.queryByTestId("sidebar-backdrop")).toBeNull();
  });

  it("calls onHide when the backdrop is clicked", () => {
    mockMatchMedia(true);
    const onHide = vi.fn();
    renderSidebar({ hidden: false, onHide });
    fireEvent.click(screen.getByTestId("sidebar-backdrop"));
    expect(onHide).toHaveBeenCalledTimes(1);
  });

  it("auto-closes the sidebar when a nav link is clicked on mobile", () => {
    mockMatchMedia(true);
    const onHide = vi.fn();
    renderSidebar({ hidden: false, onHide });
    fireEvent.click(screen.getByTestId("sidebar-issues-all"));
    expect(onHide).toHaveBeenCalledTimes(1);
  });

  it("does NOT auto-close when a section-toggle button is clicked on mobile", () => {
    mockMatchMedia(true);
    const onHide = vi.fn();
    renderSidebar({ hidden: false, onHide });
    fireEvent.click(screen.getByTestId("sidebar-section-issues"));
    expect(onHide).not.toHaveBeenCalled();
  });

  it("does NOT auto-close when a nav link is clicked on desktop", () => {
    mockMatchMedia(false);
    const onHide = vi.fn();
    renderSidebar({ hidden: false, onHide });
    fireEvent.click(screen.getByTestId("sidebar-issues-all"));
    expect(onHide).not.toHaveBeenCalled();
  });

  it("closes on Escape key when on mobile and visible", () => {
    mockMatchMedia(true);
    const onHide = vi.fn();
    renderSidebar({ hidden: false, onHide });
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onHide).toHaveBeenCalledTimes(1);
  });

  it("does not respond to Escape on mobile when already hidden", () => {
    mockMatchMedia(true);
    const onHide = vi.fn();
    renderSidebar({ hidden: true, onHide });
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onHide).not.toHaveBeenCalled();
  });

  it("does not respond to Escape on desktop", () => {
    mockMatchMedia(false);
    const onHide = vi.fn();
    renderSidebar({ hidden: false, onHide });
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onHide).not.toHaveBeenCalled();
  });
});

describe("Sidebar version badge", () => {
  it("renders the version badge once apiClient.getVersion resolves", async () => {
    getVersionMock.mockResolvedValue({ version: "9.9.9" });
    renderSidebar();
    const badge = await screen.findByTestId("sidebar-version");
    expect(badge.textContent).toBe("9.9.9");
  });

  it("renders nothing when apiClient.getVersion rejects", async () => {
    getVersionMock.mockRejectedValue(new Error("boom"));
    renderSidebar();
    await waitFor(() => expect(getVersionMock).toHaveBeenCalled());
    expect(screen.queryByTestId("sidebar-version")).toBeNull();
  });

  it("does not render the badge before the version has loaded", () => {
    let resolve: ((value: { version: string }) => void) | undefined;
    getVersionMock.mockReturnValue(
      new Promise<{ version: string }>((r) => {
        resolve = r;
      }),
    );
    renderSidebar();
    expect(screen.queryByTestId("sidebar-version")).toBeNull();
    // Cleanup: resolve the pending promise so React state updates don't leak.
    resolve?.({ version: "x" });
  });
});
