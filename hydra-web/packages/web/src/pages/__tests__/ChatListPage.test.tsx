import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import React from "react";

// --- Module-level mock state controlled by individual tests ---

const mockNavigate = vi.fn();
const mockCreateConversation = vi.fn();
const mockInvalidateQueries = vi.fn();

type MutationCallbacks = {
  mutationFn: () => Promise<unknown>;
  onSuccess?: (data: unknown) => void;
  onError?: (err: Error) => void;
};

let mutationOutcome: "success" | "error" | "pending" = "success";
let mutationError: Error | null = null;

vi.mock("react-router-dom", () => ({
  useNavigate: () => mockNavigate,
  Link: ({ children, to }: { children: React.ReactNode; to: string }) => (
    <a href={to}>{children}</a>
  ),
}));

vi.mock("@tanstack/react-query", () => ({
  useMutation: ({ mutationFn, onSuccess, onError }: MutationCallbacks) => ({
    mutate: () => {
      const promise = mutationFn();
      if (mutationOutcome === "success") {
        promise.then((data) => onSuccess?.(data));
      } else if (mutationOutcome === "error") {
        promise.catch((err) => onError?.(err));
      }
    },
    isPending: mutationOutcome === "pending",
    error: mutationOutcome === "error" ? mutationError : null,
  }),
  useQueryClient: () => ({ invalidateQueries: mockInvalidateQueries }),
}));

vi.mock("../../api/client", () => ({
  apiClient: { createConversation: mockCreateConversation },
}));

vi.mock("../../features/chat/useConversations", () => ({
  useConversations: () => ({
    data: [],
    isLoading: false,
    error: null,
    refetch: vi.fn(),
  }),
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
}));

vi.mock("../../components/LoadingState/LoadingState", () => ({
  LoadingState: () => <div data-testid="loading" />,
}));

vi.mock("../../components/ErrorState/ErrorState", () => ({
  ErrorState: ({ message }: { message: string }) => (
    <div data-testid="error">{message}</div>
  ),
}));

vi.mock("../../components/EmptyState/EmptyState", () => ({
  EmptyState: ({ message }: { message: string }) => (
    <div data-testid="empty">{message}</div>
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

// --- Import after mocks ---
const { ChatListPage } = await import("../ChatListPage");

// --- Tests ---

describe("ChatListPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mutationOutcome = "success";
    mutationError = null;
    mockCreateConversation.mockReset();
  });

  it("creates a conversation with {} and navigates to the new id on click", async () => {
    mockCreateConversation.mockResolvedValue({ conversation_id: "c-new" });

    render(<ChatListPage />);
    fireEvent.click(screen.getByText("New Chat"));

    // Allow the resolved promise's .then to flush.
    await Promise.resolve();
    await Promise.resolve();

    expect(mockCreateConversation).toHaveBeenCalledTimes(1);
    expect(mockCreateConversation).toHaveBeenCalledWith({});
    expect(mockInvalidateQueries).toHaveBeenCalledWith({ queryKey: ["conversations"] });
    expect(mockNavigate).toHaveBeenCalledWith("/chat/c-new");
  });

  it("disables the button and shows Creating… while the mutation is pending", () => {
    mutationOutcome = "pending";
    render(<ChatListPage />);
    const btn = screen.getByText("Creating…") as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
    expect(screen.queryByText("New Chat")).toBeNull();
  });

  it("renders an error message when the create mutation fails", () => {
    mutationOutcome = "error";
    mutationError = new Error("boom");
    render(<ChatListPage />);
    expect(screen.getByTestId("error").textContent).toContain("Failed to create conversation");
    expect(screen.getByTestId("error").textContent).toContain("boom");
  });
});
