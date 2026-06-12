import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import React from "react";
import type { ConversationSummary } from "@hydra/api";

// --- Mocks ---

const mockNavigate = vi.fn();

let searchParamsString = "";
const setSearchParamsMock = vi.fn(
  (
    updater:
      | URLSearchParams
      | string
      | Record<string, string>
      | ((prev: URLSearchParams) => URLSearchParams),
  ) => {
    const prev = new URLSearchParams(searchParamsString);
    let next: URLSearchParams;
    if (typeof updater === "function") {
      next = updater(prev);
    } else if (updater instanceof URLSearchParams) {
      next = updater;
    } else if (typeof updater === "string") {
      next = new URLSearchParams(updater);
    } else {
      next = new URLSearchParams(updater);
    }
    searchParamsString = next.toString();
  },
);

vi.mock("react-router-dom", () => ({
  useNavigate: () => mockNavigate,
  useSearchParams: () => {
    return [new URLSearchParams(searchParamsString), setSearchParamsMock] as const;
  },
  Link: ({ to, children, className }: {
    to: string;
    children: React.ReactNode;
    className?: string;
  }) => (
    <a href={to} className={className}>
      {children}
    </a>
  ),
}));

const openChatCreateMock = vi.fn();
vi.mock("../../features/chat/useChatCreateModal", () => ({
  useChatCreateModal: () => ({
    isOpen: false,
    open: openChatCreateMock,
    close: vi.fn(),
  }),
}));

let mockConversations: ConversationSummary[] = [];
const mockRefetch = vi.fn();
const useConversationsMock = vi.fn();

vi.mock("../../features/chat/useConversations", () => ({
  useConversations: (...args: unknown[]) => {
    useConversationsMock(...args);
    return {
      data: mockConversations,
      isLoading: false,
      error: null,
      refetch: mockRefetch,
    };
  },
}));

let mockUser: { actor: { type: "user"; username: string } } | null = {
  actor: { type: "user", username: "alice" },
};
vi.mock("../../features/auth/useAuth", () => ({
  useAuth: () => ({ user: mockUser, logout: vi.fn(), loading: false }),
}));

vi.mock("../../api/auth", () => ({
  actorDisplayName: (actor: { type: string; username?: string }) =>
    actor.type === "user" ? actor.username : "",
}));

// The ChatListPage now imports FilterBar from features/filters. Stub it to a
// no-op div so this file stays focused on URL ↔ server-query wiring and the
// auto-creator seed behaviour. End-to-end chip interactions are covered in
// the @chat:filter-bar Playwright spec.
vi.mock("../../features/filters", () => ({
  FilterBar: () => <div data-testid="filter-bar" />,
}));

vi.mock("../../features/chat/conversationFilters", () => ({
  useConversationFilters: () => ({}),
}));

vi.mock("../../features/chat/conversationStatusBadge", () => ({
  CONVERSATION_STATUS_TONES: {
    active: "conv-active",
    idle: "conv-idle",
    closed: "conv-closed",
    unknown: "unknown",
  },
  CONVERSATION_STATUS_LABELS: {
    active: "Active",
    idle: "Idle",
    closed: "Closed",
    unknown: "Unknown",
  },
}));

const BADGE_LABELS: Record<string, string> = {
  "conv-active": "Active",
  "conv-idle": "Idle",
  "conv-closed": "Closed",
};

vi.mock("@hydra/ui", () => ({
  Badge: ({ status }: { status: string }) => (
    <span data-testid="badge" data-status={status}>
      {BADGE_LABELS[status] ?? status}
    </span>
  ),
  Button: ({
    children,
    onClick,
    disabled,
  }: {
    children: React.ReactNode;
    onClick?: () => void;
    disabled?: boolean;
    variant?: string;
    size?: string;
  }) => (
    <button onClick={onClick} disabled={disabled}>
      {children}
    </button>
  ),
  Icons: new Proxy(
    {},
    {
      get: (_t, prop) => () => <span data-testid={`icon-${String(prop)}`} />,
    },
  ),
}));

vi.mock("../../utils/time", () => ({
  formatRelativeTime: (s: string) => s,
  shortRelativeTime: (s: string) => s,
}));

vi.mock("../ChatListPage.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const useBreadcrumbsMock = vi.fn();
vi.mock("../../layout/useBreadcrumbs", () => ({
  useBreadcrumbs: (...args: unknown[]) => useBreadcrumbsMock(...args),
}));

// --- Import after mocks ---
const { ChatListPage } = await import("../ChatListPage");

// --- Helpers ---

function resetState() {
  vi.clearAllMocks();
  mockConversations = [];
  openChatCreateMock.mockReset();
  useBreadcrumbsMock.mockReset();
  useConversationsMock.mockReset();
  setSearchParamsMock.mockClear();
  searchParamsString = "";
  mockUser = { actor: { type: "user", username: "alice" } };
}

// --- Tests ---

describe("ChatListPage New Chat button", () => {
  beforeEach(resetState);

  it("publishes a Workspace / Chats breadcrumb on mount", () => {
    render(<ChatListPage />);
    expect(useBreadcrumbsMock).toHaveBeenCalledWith(
      [{ label: "Workspace", to: "/" }],
      "Chats",
    );
    cleanup();
  });

  it("opens the chat-create modal on click instead of creating directly", () => {
    render(<ChatListPage />);

    const button = screen.getByRole("button", { name: /new chat/i });
    fireEvent.click(button);

    expect(openChatCreateMock).toHaveBeenCalledTimes(1);
    cleanup();
  });

  it("orders rows by status bucket (active > idle > closed), then updated_at desc within each bucket", () => {
    mockConversations = [
      {
        conversation_id: "c-old-closed",
        title: "Old closed",
        agent_name: null,
        status: "closed",
        event_count: 0,
        last_event_preview: null,
        creator: "alice",
        created_at: "2026-05-19T00:00:00Z",
        updated_at: "2026-05-18T18:00:00Z",
      },
      {
        conversation_id: "c-new-closed",
        title: "New closed",
        agent_name: null,
        status: "closed",
        event_count: 0,
        last_event_preview: null,
        creator: "alice",
        created_at: "2026-05-19T00:00:00Z",
        updated_at: "2026-05-19T00:00:00Z",
      },
      {
        conversation_id: "c-old-active",
        title: "Old active",
        agent_name: null,
        status: "active",
        event_count: 0,
        last_event_preview: null,
        creator: "alice",
        created_at: "2026-05-18T19:00:00Z",
        updated_at: "2026-05-18T19:00:00Z",
      },
      {
        conversation_id: "c-new-idle",
        title: "New idle",
        agent_name: null,
        status: "idle",
        event_count: 0,
        last_event_preview: null,
        creator: "alice",
        created_at: "2026-05-18T23:30:00Z",
        updated_at: "2026-05-18T23:30:00Z",
      },
      {
        conversation_id: "c-old-idle",
        title: "Old idle",
        agent_name: null,
        status: "idle",
        event_count: 0,
        last_event_preview: null,
        creator: "alice",
        created_at: "2026-05-18T20:00:00Z",
        updated_at: "2026-05-18T20:00:00Z",
      },
      {
        conversation_id: "c-new-active",
        title: "New active",
        agent_name: null,
        status: "active",
        event_count: 0,
        last_event_preview: null,
        creator: "alice",
        created_at: "2026-05-18T23:00:00Z",
        updated_at: "2026-05-18T23:00:00Z",
      },
    ];
    render(<ChatListPage />);

    const rows = screen.getAllByTestId(/^chats-list-row-/);
    expect(rows.map((r) => r.getAttribute("data-testid"))).toEqual([
      "chats-list-row-c-new-active",
      "chats-list-row-c-old-active",
      "chats-list-row-c-new-idle",
      "chats-list-row-c-old-idle",
      "chats-list-row-c-new-closed",
      "chats-list-row-c-old-closed",
    ]);

    cleanup();
  });

  it("renders the Messages count as the sum across sessions (multi-session conversation)", () => {
    mockConversations = [
      {
        conversation_id: "c-multi",
        title: "Two-session chat",
        agent_name: null,
        status: "active",
        event_count: 11,
        last_event_preview: "Assistant: …",
        creator: "alice",
        created_at: "2026-05-19T00:00:00Z",
        updated_at: "2026-05-19T00:00:00Z",
      },
      {
        conversation_id: "c-single",
        title: "Single-session chat",
        agent_name: null,
        status: "active",
        event_count: 3,
        last_event_preview: "User: hi",
        creator: "alice",
        created_at: "2026-05-19T00:00:00Z",
        updated_at: "2026-05-18T23:00:00Z",
      },
    ];
    render(<ChatListPage />);

    const multiRow = screen.getByTestId("chats-list-row-c-multi");
    expect(multiRow.textContent).toContain("11");
    const singleRow = screen.getByTestId("chats-list-row-c-single");
    expect(singleRow.textContent).toContain("3");

    cleanup();
  });

  it("renders literal Active / Idle / Closed status labels on chat rows", () => {
    mockConversations = [
      {
        conversation_id: "c-active",
        title: "An active chat",
        agent_name: null,
        status: "active",
        event_count: 1,
        last_event_preview: null,
        creator: "alice",
        created_at: "2026-05-19T00:00:00Z",
        updated_at: "2026-05-19T00:00:00Z",
      },
      {
        conversation_id: "c-idle",
        title: "An idle chat",
        agent_name: null,
        status: "idle",
        event_count: 1,
        last_event_preview: null,
        creator: "alice",
        created_at: "2026-05-19T00:00:00Z",
        updated_at: "2026-05-18T23:00:00Z",
      },
      {
        conversation_id: "c-closed",
        title: "A closed chat",
        agent_name: null,
        status: "closed",
        event_count: 1,
        last_event_preview: null,
        creator: "alice",
        created_at: "2026-05-19T00:00:00Z",
        updated_at: "2026-05-18T22:00:00Z",
      },
    ];
    render(<ChatListPage />);

    const badges = screen.getAllByTestId("badge");
    const labels = badges.map((b) => b.textContent);
    expect(labels).toEqual(["Active", "Idle", "Closed"]);

    const statuses = badges.map((b) => b.getAttribute("data-status"));
    expect(statuses).toEqual(["conv-active", "conv-idle", "conv-closed"]);

    cleanup();
  });
});

describe("ChatListPage default creator filter", () => {
  beforeEach(resetState);

  it("seeds ?creator=users/<me> on first visit and queries with creator=<me>", () => {
    render(<ChatListPage />);

    // The page writes the seeded creator chip back to the URL via replace.
    expect(setSearchParamsMock).toHaveBeenCalled();
    expect(searchParamsString).toBe("creator=users%2Falice");

    // The server query carries `creator=alice` (Principal path stripped).
    expect(useConversationsMock).toHaveBeenCalled();
    const lastArg =
      useConversationsMock.mock.calls[useConversationsMock.mock.calls.length - 1]?.[0];
    expect(lastArg).toEqual({ creator: "alice" });
    cleanup();
  });

  it("does not seed a creator filter when the user is not authenticated", () => {
    mockUser = null;
    render(<ChatListPage />);
    expect(searchParamsString).toBe("");
    const lastArg =
      useConversationsMock.mock.calls[useConversationsMock.mock.calls.length - 1]?.[0];
    expect(lastArg).toEqual({});
    cleanup();
  });

  it("respects an explicit ?creator= in the URL without re-seeding", () => {
    searchParamsString = "creator=users/bob";
    render(<ChatListPage />);
    expect(searchParamsString).toBe("creator=users/bob");
    const lastArg =
      useConversationsMock.mock.calls[useConversationsMock.mock.calls.length - 1]?.[0];
    expect(lastArg).toEqual({ creator: "bob" });
    cleanup();
  });

  it("resolves legacy ?scope=mine into a creator chip", () => {
    searchParamsString = "scope=mine";
    render(<ChatListPage />);
    expect(searchParamsString).toBe("creator=users%2Falice");
    const lastArg =
      useConversationsMock.mock.calls[useConversationsMock.mock.calls.length - 1]?.[0];
    expect(lastArg).toEqual({ creator: "alice" });
    cleanup();
  });

  it("resolves legacy ?scope=all into no creator filter", () => {
    searchParamsString = "scope=all";
    render(<ChatListPage />);
    expect(searchParamsString).toBe("");
    const lastArg =
      useConversationsMock.mock.calls[useConversationsMock.mock.calls.length - 1]?.[0];
    expect(lastArg).toEqual({});
    cleanup();
  });

  it("respects an explicit ?status= without seeding a creator filter", () => {
    searchParamsString = "status=closed";
    render(<ChatListPage />);
    expect(searchParamsString).toBe("status=closed");
    const lastArg =
      useConversationsMock.mock.calls[useConversationsMock.mock.calls.length - 1]?.[0];
    expect(lastArg).toEqual({ status: "closed" });
    cleanup();
  });
});
