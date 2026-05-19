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
vi.mock("../features/chat/useConversations", () => ({
  useConversations: () => ({
    data: mockConversations,
    isLoading: false,
    error: null,
  }),
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
    const issuesLink = screen.getByTestId("sidebar-issues-all") as HTMLAnchorElement;
    expect(issuesLink).toBeTruthy();
    expect(issuesLink.getAttribute("href")).toBe("/?selected=all");
    expect(screen.getByTestId("sidebar-patches")).toBeTruthy();
    expect(screen.getByTestId("sidebar-sessions")).toBeTruthy();
    expect(screen.getByTestId("sidebar-chats")).toBeTruthy();
    expect(screen.getByTestId("sidebar-documents")).toBeTruthy();
    expect(screen.getByTestId("sidebar-agents")).toBeTruthy();
    expect(screen.getByTestId("sidebar-context-repositories")).toBeTruthy();
    expect(screen.getByTestId("sidebar-context-secrets")).toBeTruthy();
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
    expect(link.getAttribute("href")).toBe("/?selected=assigned");
    expect(screen.getByTestId("sidebar-issues-assigned-badge").textContent).toBe("4");
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

  it("excludes closed chats and orders the rest by most-recently-updated", () => {
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
    ];
    renderSidebar();
    expect(screen.queryByTestId("sidebar-chat-row-c-recent-closed")).toBeNull();
    const rows = screen.getAllByTestId(/^sidebar-chat-row-/);
    expect(rows.map((r) => r.getAttribute("data-testid"))).toEqual([
      "sidebar-chat-row-c-recent-idle",
      "sidebar-chat-row-c-old-active",
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
