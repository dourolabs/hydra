import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import React from "react";
import type { ConversationSummary } from "@hydra/api";

// --- Mocks ---

const mockNavigate = vi.fn();
vi.mock("react-router-dom", () => ({
  useNavigate: () => mockNavigate,
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

const mockInvalidateQueries = vi.fn();

type MutationState = {
  isPending: boolean;
  error: Error | null;
};

const mutationState: MutationState = {
  isPending: false,
  error: null,
};

vi.mock("@tanstack/react-query", () => ({
  useMutation: ({
    mutationFn,
    onSuccess,
  }: {
    mutationFn: () => Promise<unknown>;
    onSuccess?: (data: unknown) => void;
  }) => ({
    mutate: () => {
      mutationFn().then((data) => {
        onSuccess?.(data);
      });
    },
    isPending: mutationState.isPending,
    error: mutationState.error,
  }),
  useQueryClient: () => ({
    invalidateQueries: mockInvalidateQueries,
  }),
}));

let mockConversations: ConversationSummary[] = [];
const mockRefetch = vi.fn();

vi.mock("../../features/chat/useConversations", () => ({
  useConversations: () => ({
    data: mockConversations,
    isLoading: false,
    error: null,
    refetch: mockRefetch,
  }),
}));

const mockCreateConversation = vi.fn();
vi.mock("../../api/client", () => ({
  apiClient: {
    createConversation: (...args: unknown[]) => mockCreateConversation(...args),
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

vi.mock("../../components/LoadingState/LoadingState", () => ({
  LoadingState: () => <div data-testid="loading" />,
}));

vi.mock("../../components/ErrorState/ErrorState", () => ({
  ErrorState: ({ message }: { message: string }) => (
    <div data-testid="error-state">{message}</div>
  ),
}));

vi.mock("../../components/EmptyState/EmptyState", () => ({
  EmptyState: ({ message }: { message: string }) => (
    <div data-testid="empty-state">{message}</div>
  ),
}));

vi.mock("../../utils/time", () => ({
  formatRelativeTime: (s: string) => s,
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

function resetMutationState() {
  mutationState.isPending = false;
  mutationState.error = null;
}

// --- Tests ---

describe("ChatListPage New Chat button", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockConversations = [];
    resetMutationState();
    mockCreateConversation.mockReset();
    useBreadcrumbsMock.mockReset();
  });

  it("publishes a Workspace / Chats breadcrumb on mount", () => {
    render(<ChatListPage />);
    expect(useBreadcrumbsMock).toHaveBeenCalledWith(
      [{ label: "Workspace", to: "/" }],
      "Chats",
    );
    cleanup();
  });

  it("creates a conversation and navigates on click", async () => {
    mockCreateConversation.mockResolvedValue({ conversation_id: "c-new123" });
    render(<ChatListPage />);

    const button = screen.getByRole("button", { name: /new chat/i });
    fireEvent.click(button);

    expect(mockCreateConversation).toHaveBeenCalledTimes(1);
    expect(mockCreateConversation).toHaveBeenCalledWith({});

    // Allow the promise chain in the mocked mutate() to settle.
    await Promise.resolve();
    await Promise.resolve();

    expect(mockInvalidateQueries).toHaveBeenCalledWith({
      queryKey: ["conversations"],
    });
    expect(mockNavigate).toHaveBeenCalledWith("/chat/c-new123");

    cleanup();
  });

  it("shows Creating… and disables the button while the mutation is pending", () => {
    mutationState.isPending = true;
    render(<ChatListPage />);

    const button = screen.getByRole("button", { name: /creating/i });
    expect(button).toBeDefined();
    expect((button as HTMLButtonElement).disabled).toBe(true);

    cleanup();
  });

  it("renders an error banner when the create mutation has an error", () => {
    mutationState.error = new Error("network down");
    render(<ChatListPage />);

    expect(screen.getByText(/network down/)).toBeDefined();

    cleanup();
  });

  it("orders rows by status bucket (active > idle > closed), then updated_at desc within each bucket", () => {
    // Timestamps are chosen so that bucket-only and recency-only orderings diverge:
    // c-new-closed is the single most recently updated, but must sink to the bottom;
    // c-old-active is the least recently updated active, but must still rank above
    // any idle (incl. the most-recently-updated idle).
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
