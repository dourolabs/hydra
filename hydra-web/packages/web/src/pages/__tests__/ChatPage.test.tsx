import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import React from "react";
import type { Conversation, ConversationEvent } from "@hydra/api";

// --- Mocks ---

vi.mock("react-router-dom", () => ({
  useParams: () => ({ conversationId: "c-test123" }),
  useNavigate: () => vi.fn(),
}));

let mockConversation: Conversation | undefined;
let mockEvents: ConversationEvent[] = [];
let mockIsLoading = false;
let mockError: Error | null = null;

vi.mock("../../features/chat/useConversations", () => ({
  useConversation: () => ({
    data: mockConversation,
    isLoading: mockIsLoading,
    error: mockError,
  }),
  useConversationEvents: () => ({
    data: mockEvents,
  }),
}));

vi.mock("@tanstack/react-query", () => ({
  useMutation: () => ({
    mutate: vi.fn(),
    isPending: false,
  }),
  useQueryClient: () => ({
    cancelQueries: vi.fn(),
    getQueryData: vi.fn(),
    setQueryData: vi.fn(),
    invalidateQueries: vi.fn(),
  }),
}));

// Stub the chat Related-tab data hooks so this test stays focused on layout.
vi.mock("../../features/chat/useChatActiveSessionIssues", () => ({
  useChatActiveSessionIssues: () => ({
    issues: [],
    sessionsByIssue: new Map(),
    isLoading: false,
  }),
}));
vi.mock("../../features/chat/useChatAttentionIssues", () => ({
  useChatAttentionIssues: () => ({ issues: [], isLoading: false }),
}));
vi.mock("../../features/chat/useChatTopLevelIssues", () => ({
  useChatTopLevelIssues: () => ({ issues: [], isLoading: false }),
}));
vi.mock("../../features/chat/useChatRelatedDocuments", () => ({
  useChatRelatedDocuments: () => ({ documents: [], isLoading: false }),
}));
vi.mock("../../features/chat/useChatRelatedPatches", () => ({
  useChatRelatedPatches: () => ({ patches: [], isLoading: false }),
}));

vi.mock("../../api/client", () => ({
  apiClient: {
    sendMessage: vi.fn(),
    closeConversation: vi.fn(),
  },
  ApiError: class ApiError extends Error {
    status: number;
    constructor(message: string, status: number) {
      super(message);
      this.status = status;
    }
  },
}));

vi.mock("@hydra/ui", () => ({
  Button: ({
    children,
    onClick,
    disabled,
  }: {
    children: React.ReactNode;
    onClick?: () => void;
    disabled?: boolean;
  }) => (
    <button onClick={onClick} disabled={disabled}>
      {children}
    </button>
  ),
  Panel: ({
    header,
    children,
  }: {
    header?: React.ReactNode;
    children: React.ReactNode;
    className?: string;
  }) => (
    <div>
      {header && <div>{header}</div>}
      <div>{children}</div>
    </div>
  ),
  Spinner: () => <div data-testid="spinner" />,
  Tabs: ({
    tabs,
    activeTab,
    onTabChange,
  }: {
    tabs: { id: string; label: React.ReactNode }[];
    activeTab: string;
    onTabChange: (id: string) => void;
  }) => (
    <div role="tablist">
      {tabs.map((tab) => (
        <button
          key={tab.id}
          role="tab"
          aria-selected={tab.id === activeTab}
          onClick={() => onTabChange(tab.id)}
        >
          {tab.label}
        </button>
      ))}
    </div>
  ),
  Tooltip: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  MarkdownViewer: ({ content }: { content: string }) => <div>{content}</div>,
}));

// CSS Module proxies
const cssProxy = new Proxy({}, { get: (_t, prop) => String(prop) });
vi.mock("../ChatPage.module.css", () => ({ default: cssProxy }));
vi.mock("../../features/chat/ChatHeader.module.css", () => ({ default: cssProxy }));
vi.mock("../../features/chat/ChatInput.module.css", () => ({ default: cssProxy }));
vi.mock("../../features/chat/ChatMessageList.module.css", () => ({ default: cssProxy }));
vi.mock("../../features/chat/ChatRightPanel.module.css", () => ({ default: cssProxy }));
vi.mock("../../features/chat/ChatMetadataTab.module.css", () => ({ default: cssProxy }));
vi.mock("../../features/chat/ChatRelatedTab.module.css", () => ({ default: cssProxy }));
vi.mock("../../features/issues/IssueSettings.module.css", () => ({ default: cssProxy }));

vi.mock("../../utils/time", () => ({
  formatTimestamp: (s: string) => s,
}));

// --- Import after mocks ---
const { ChatPage } = await import("../ChatPage");

// --- Helpers ---

function makeConversation(overrides: Partial<Conversation> = {}): Conversation {
  return {
    conversation_id: "c-test123",
    title: "Test Conversation",
    agent_name: "claude-test",
    creator: "alice",
    status: "open",
    created_at: "2026-05-01T00:00:00Z",
    updated_at: "2026-05-02T00:00:00Z",
    session_settings: { repo_name: "dourolabs/hydra" },
    ...overrides,
  } as Conversation;
}

// --- Tests ---

describe("ChatPage 2-pane layout", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockConversation = makeConversation();
    mockEvents = [];
    mockIsLoading = false;
    mockError = null;
  });

  it("renders Related and Metadata tabs in the right panel", () => {
    render(<ChatPage />);

    expect(screen.getByRole("tab", { name: "Related" })).toBeDefined();
    expect(screen.getByRole("tab", { name: "Metadata" })).toBeDefined();

    cleanup();
  });

  it("shows the chat input in the main pane regardless of active right-panel tab", () => {
    render(<ChatPage />);

    // Chat input is visible by default (Related tab is active).
    expect(screen.getByPlaceholderText("Type a message...")).toBeDefined();

    // Switch to Metadata.
    fireEvent.click(screen.getByRole("tab", { name: "Metadata" }));

    // Chat input is still visible.
    expect(screen.getByPlaceholderText("Type a message...")).toBeDefined();

    cleanup();
  });

  it("reveals Conversation ID when switching to the Metadata tab", () => {
    render(<ChatPage />);

    // Metadata content is not visible on the Related tab.
    expect(screen.queryByText("c-test123")).toBeNull();

    fireEvent.click(screen.getByRole("tab", { name: "Metadata" }));

    expect(screen.getByText("Conversation ID")).toBeDefined();
    expect(screen.getByText("c-test123")).toBeDefined();

    cleanup();
  });

  it("renders all 5 Related section headings with empty placeholders", () => {
    render(<ChatPage />);

    const headings = [
      "Issues with active sessions",
      "Needs my attention",
      "Top-level issues",
      "Documents",
      "Patches",
    ];
    for (const heading of headings) {
      expect(screen.getByText(heading)).toBeDefined();
    }

    // Each section has an "(empty)" placeholder.
    expect(screen.getAllByText("(empty)").length).toBe(headings.length);

    cleanup();
  });

  it("shows a spinner while the conversation is loading", () => {
    mockIsLoading = true;
    mockConversation = undefined;
    render(<ChatPage />);

    expect(screen.getByTestId("spinner")).toBeDefined();
    expect(screen.queryByRole("tab", { name: "Related" })).toBeNull();

    cleanup();
  });

  it("shows an error message when the conversation fails to load", () => {
    mockError = new Error("boom");
    mockConversation = undefined;
    render(<ChatPage />);

    expect(screen.getByText(/Failed to load conversation: boom/)).toBeDefined();

    cleanup();
  });
});
