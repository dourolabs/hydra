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

vi.mock("@hydra/ui", () => ({
  Badge: ({ status }: { status: string }) => <span data-testid="badge">{status}</span>,
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

vi.mock("../../utils/statusMapping", () => ({
  normalizeConversationStatus: (s: string) => s,
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
});
