// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import React from "react";
import { MemoryRouter, useLocation } from "react-router-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ConversationSummary } from "@hydra/api";

// --- Mocks ---

vi.mock("@hydra/ui", () => ({
  Avatar: ({ name }: { name: string }) => <div data-testid="avatar">{name}</div>,
  Tooltip: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  Kbd: ({ children }: { children: React.ReactNode }) => <kbd>{children}</kbd>,
  HydraMark: () => <span data-testid="hydra-mark" />,
  Icons: new Proxy(
    {},
    {
      get: (_t, prop) => () => <span data-testid={`icon-${String(prop)}`} />,
    },
  ),
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
const useConversationsMock = vi.fn();
vi.mock("../features/chat/useConversations", () => ({
  useConversations: (...args: unknown[]) => {
    useConversationsMock(...args);
    return {
      data: mockConversations,
      isLoading: false,
      error: null,
    };
  },
}));

const issueCountMock = vi.fn();
vi.mock("../features/issues/usePaginatedIssues", () => ({
  useIssueCount: (...args: unknown[]) => issueCountMock(...args),
}));

const activeSessionsMock = vi.fn();
vi.mock("../features/sessions/useActiveSessions", () => ({
  useActiveSessions: (...args: unknown[]) => activeSessionsMock(...args),
}));

const activeSessionCountMock = vi.fn();
vi.mock("../features/sessions/useActiveSessionCount", () => ({
  useActiveSessionCount: (...args: unknown[]) => activeSessionCountMock(...args),
}));

const getVersionMock = vi.fn();
vi.mock("../api/client", () => ({
  apiClient: {
    getVersion: (...args: unknown[]) => getVersionMock(...args),
  },
}));

vi.mock("./Sidebar.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

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
    onOpenSearch?: () => void;
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
          onOpenSearch={overrides.onOpenSearch ?? (() => {})}
        />
        <LocationDisplay />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  mockConversations = [];
  useConversationsMock.mockClear();
  issueCountMock.mockReturnValue({ data: 0, isLoading: false, error: null });
  activeSessionsMock.mockReturnValue({ data: [], isLoading: false, error: null });
  activeSessionCountMock.mockReturnValue({ data: 0, isLoading: false, error: null });
  getVersionMock.mockResolvedValue({ version: "0.0.0" });

  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    writable: true,
    value: () =>
      ({
        matches: false,
        media: "",
        addEventListener: () => {},
        removeEventListener: () => {},
        addListener: () => {},
        removeListener: () => {},
        dispatchEvent: () => true,
      }) as unknown as MediaQueryList,
  });

  Object.defineProperty(window, "localStorage", {
    configurable: true,
    writable: true,
    value: {
      getItem: () => null,
      setItem: () => undefined,
      removeItem: () => undefined,
      clear: () => undefined,
      key: () => null,
      length: 0,
    } as Storage,
  });
});

afterEach(() => {
  cleanup();
});

describe("Sidebar", () => {
  it("renders the brand and workspace nav items", () => {
    renderSidebar();
    expect(screen.getByTestId("hydra-brand")).toBeTruthy();
    // The Workspace > Issues link is the all-issues landing page.
    const allLink = screen.getByTestId("sidebar-issues-all") as HTMLAnchorElement;
    expect(allLink).toBeTruthy();
    expect(allLink.getAttribute("href")).toBe("/");
    expect(screen.getByTestId("sidebar-patches")).toBeTruthy();
    expect(screen.getByTestId("sidebar-sessions")).toBeTruthy();
    expect(screen.getByTestId("sidebar-chats")).toBeTruthy();
    expect(screen.getByTestId("sidebar-documents")).toBeTruthy();
    expect(screen.getByTestId("sidebar-agents")).toBeTruthy();
    expect(screen.getByTestId("sidebar-context-repositories")).toBeTruthy();
    expect(screen.getByTestId("sidebar-context-secrets")).toBeTruthy();
  });

  it("exposes the My issues view scoped to the current user", () => {
    renderSidebar();
    const myLink = screen.getByTestId("sidebar-issues-your-issues") as HTMLAnchorElement;
    expect(myLink.getAttribute("href")).toBe("/?creator=Alice");
  });

  it("marks Workspace > Issues active at / with no filter params", () => {
    renderSidebar({ initialEntry: "/" });
    const allLink = screen.getByTestId("sidebar-issues-all") as HTMLAnchorElement;
    expect(allLink.className).toContain("itemActive");
    const myLink = screen.getByTestId("sidebar-issues-your-issues") as HTMLAnchorElement;
    expect(myLink.className).not.toContain("itemActive");
  });

  it("marks the My issues view active at /?creator=<user>", () => {
    renderSidebar({ initialEntry: "/?creator=Alice" });
    const myLink = screen.getByTestId("sidebar-issues-your-issues") as HTMLAnchorElement;
    expect(myLink.className).toContain("itemActive");
    const allLink = screen.getByTestId("sidebar-issues-all") as HTMLAnchorElement;
    expect(allLink.className).not.toContain("itemActive");
  });

  it("highlights neither Issues nor My issues when an unrelated filter is active", () => {
    // With creator=other, neither the Workspace > Issues "all" landing
    // nor the My issues view (creator=Alice) matches — so both stay dim.
    renderSidebar({ initialEntry: "/?creator=other" });
    const myLink = screen.getByTestId("sidebar-issues-your-issues") as HTMLAnchorElement;
    expect(myLink.className).not.toContain("itemActive");
    const allLink = screen.getByTestId("sidebar-issues-all") as HTMLAnchorElement;
    expect(allLink.className).not.toContain("itemActive");
  });

  it("workspace items navigate to the expected routes", () => {
    renderSidebar();
    expect(
      (screen.getByTestId("sidebar-patches") as HTMLAnchorElement).getAttribute("href"),
    ).toBe("/patches");
    expect(
      (screen.getByTestId("sidebar-sessions") as HTMLAnchorElement).getAttribute("href"),
    ).toBe("/sessions");
    expect(
      (screen.getByTestId("sidebar-chats") as HTMLAnchorElement).getAttribute("href"),
    ).toBe("/chat");
    expect(
      (screen.getByTestId("sidebar-documents") as HTMLAnchorElement).getAttribute("href"),
    ).toBe("/documents");
    expect(
      (screen.getByTestId("sidebar-agents") as HTMLAnchorElement).getAttribute("href"),
    ).toBe("/agents");
    expect(
      (screen.getByTestId("sidebar-context-repositories") as HTMLAnchorElement).getAttribute(
        "href",
      ),
    ).toBe("/repositories");
    expect(
      (screen.getByTestId("sidebar-context-secrets") as HTMLAnchorElement).getAttribute("href"),
    ).toBe("/secrets");
  });

  it("renders the Views section with Assigned to me link", () => {
    issueCountMock.mockReturnValue({ data: 4, isLoading: false, error: null });
    renderSidebar();
    const link = screen.getByTestId("sidebar-issues-assigned") as HTMLAnchorElement;
    expect(link.getAttribute("href")).toBe("/?assignee=Alice");
    expect(screen.getByTestId("sidebar-issues-assigned-badge").textContent).toBe("4");
  });

  it("links the In progress view to the explicit status filter", () => {
    renderSidebar();
    const link = screen.getByTestId("sidebar-issues-in-progress") as HTMLAnchorElement;
    expect(link.getAttribute("href")).toBe("/?status=in-progress");
  });

  it("opens search when search button clicked", () => {
    const onOpenSearch = vi.fn();
    renderSidebar({ onOpenSearch });
    fireEvent.click(screen.getByTestId("sidebar-search"));
    expect(onOpenSearch).toHaveBeenCalled();
  });

  it("shows the user card with display name", () => {
    renderSidebar();
    expect(screen.getByTestId("avatar").textContent).toBe("Alice");
  });

  it("passes the current-user creator filter into the Chats useConversations call", () => {
    renderSidebar();
    expect(useConversationsMock).toHaveBeenCalled();
    const firstArg = useConversationsMock.mock.calls[0]?.[0];
    const secondArg = useConversationsMock.mock.calls[0]?.[1];
    expect(firstArg).toEqual({ creator: "Alice" });
    expect(secondArg).toEqual({ enabled: true });
  });

  it("renders recent chats in Chats section", () => {
    mockConversations = [
      {
        conversation_id: "c-1",
        title: "First chat",
        agent_name: null,
        status: "active",
        event_count: 0,
        last_event_preview: null,
        creator: "alice",
        created_at: new Date(Date.now() - 60_000).toISOString(),
        updated_at: new Date(Date.now() - 60_000).toISOString(),
      },
    ];
    renderSidebar();
    expect(screen.getByTestId("sidebar-chat-row-c-1")).toBeTruthy();
  });

  it("excludes closed chats and orders by status bucket (active > idle) then updated_at desc", () => {
    // c-recent-idle is the most recently updated non-closed conversation, but bucket-first
    // ordering means c-old-active (older but active) ranks above it. c-recent-closed must
    // still be excluded by the sidebar's closed filter.
    const now = Date.now();
    mockConversations = [
      {
        conversation_id: "c-old-active",
        title: "Old active",
        agent_name: null,
        status: "active",
        event_count: 0,
        last_event_preview: null,
        creator: "alice",
        created_at: new Date(now - 60 * 60_000).toISOString(),
        updated_at: new Date(now - 60 * 60_000).toISOString(),
      },
      {
        conversation_id: "c-recent-closed",
        title: "Recently closed",
        agent_name: null,
        status: "closed",
        event_count: 0,
        last_event_preview: null,
        creator: "alice",
        created_at: new Date(now - 30_000).toISOString(),
        updated_at: new Date(now - 30_000).toISOString(),
      },
      {
        conversation_id: "c-recent-idle",
        title: "Recently idle",
        agent_name: null,
        status: "idle",
        event_count: 0,
        last_event_preview: null,
        creator: "alice",
        created_at: new Date(now - 10_000).toISOString(),
        updated_at: new Date(now - 10_000).toISOString(),
      },
      {
        conversation_id: "c-newer-active",
        title: "Newer active",
        agent_name: null,
        status: "active",
        event_count: 0,
        last_event_preview: null,
        creator: "alice",
        created_at: new Date(now - 5 * 60_000).toISOString(),
        updated_at: new Date(now - 5 * 60_000).toISOString(),
      },
      {
        conversation_id: "c-older-idle",
        title: "Older idle",
        agent_name: null,
        status: "idle",
        event_count: 0,
        last_event_preview: null,
        creator: "alice",
        created_at: new Date(now - 90 * 60_000).toISOString(),
        updated_at: new Date(now - 90 * 60_000).toISOString(),
      },
    ];
    renderSidebar();
    expect(screen.queryByTestId("sidebar-chat-row-c-recent-closed")).toBeNull();
    const rows = screen.getAllByTestId(/^sidebar-chat-row-/);
    expect(rows.map((r) => r.getAttribute("data-testid"))).toEqual([
      "sidebar-chat-row-c-newer-active",
      "sidebar-chat-row-c-old-active",
      "sidebar-chat-row-c-recent-idle",
      "sidebar-chat-row-c-older-idle",
    ]);
  });

  it("renders Active sessions section with running sessions", () => {
    activeSessionsMock.mockReturnValue({
      data: [
        {
          session_id: "s-1",
          version: 1n,
          timestamp: new Date().toISOString(),
          session: {
            prompt: "Run regression suite",
            creator: "alice",
            status: "running",
            start_time: new Date(Date.now() - 60_000).toISOString(),
          },
        },
      ],
      isLoading: false,
      error: null,
    });
    renderSidebar();
    expect(screen.getByTestId("sidebar-active-sessions")).toBeTruthy();
    const row = screen.getByTestId("sidebar-session-row-s-1") as HTMLAnchorElement;
    expect(row.getAttribute("href")).toBe("/sessions/s-1");
  });

  it("renders empty state when there are no active sessions", () => {
    activeSessionsMock.mockReturnValue({
      data: [],
      isLoading: false,
      error: null,
    });
    renderSidebar();
    expect(screen.getByText("No active sessions.")).toBeTruthy();
  });

  it("shows true active session count even when more rows exist than the list cap", () => {
    // 6 rendered rows but 12 total active sessions — count badge should show 12.
    const rows = Array.from({ length: 6 }, (_, i) => ({
      session_id: `s-${i + 1}`,
      version: 1n,
      timestamp: new Date().toISOString(),
      session: {
        prompt: `Session ${i + 1}`,
        creator: "alice",
        status: "running",
        start_time: new Date(Date.now() - 60_000).toISOString(),
      },
    }));
    activeSessionsMock.mockReturnValue({ data: rows, isLoading: false, error: null });
    activeSessionCountMock.mockReturnValue({ data: 12, isLoading: false, error: null });

    renderSidebar();
    expect(screen.getByTestId("sidebar-active-sessions-count").textContent).toBe("12");
    expect(screen.getAllByTestId(/^sidebar-session-row-/).length).toBe(6);
  });

  it("hides the count badge when there are no active sessions", () => {
    activeSessionCountMock.mockReturnValue({ data: 0, isLoading: false, error: null });
    renderSidebar();
    expect(screen.queryByTestId("sidebar-active-sessions-count")).toBeNull();
  });
});
