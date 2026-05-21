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

// Stub the chat Related-tab data hook so this test stays focused on layout.
vi.mock("../../features/chat/useChatReferencedArtifacts", () => ({
  useChatReferencedArtifacts: () => ({
    issues: [],
    patches: [],
    documents: [],
    sessionsByIssue: new Map(),
    isLoading: false,
    error: null,
    hasNextPage: { issues: false, patches: false, documents: false },
    isFetchingNextPage: { issues: false, patches: false, documents: false },
    fetchNextPage: {
      issues: () => {},
      patches: () => {},
      documents: () => {},
    },
  }),
}));

// ChatRelatedTab also hydrates issue child statuses for progress bars; stub
// it here so this layout test doesn't need the @tanstack/react-query mock to
// expose useQuery.
vi.mock("../../features/dashboard/usePageIssueTrees", () => ({
  usePageIssueTrees: () => ({
    treeDataMap: new Map(),
    isActiveMap: new Map(),
    childStatusMap: new Map(),
    sessionsByIssue: new Map(),
    isLoading: false,
  }),
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
  Kbd: ({ children }: { children: React.ReactNode }) => <kbd>{children}</kbd>,
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
vi.mock("../../components/MobileTabBar.module.css", () => ({ default: cssProxy }));

vi.mock("../../utils/time", () => ({
  formatTimestamp: (s: string) => s,
  formatRelativeTime: (s: string) => s,
  shortRelativeTime: (s: string) => s,
}));

vi.mock("../../layout/useBreadcrumbs", () => ({
  useBreadcrumbs: () => {},
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
    // jsdom doesn't implement Element.scrollTo, which the ChatMessageList
    // auto-scroll effect calls.
    Element.prototype.scrollTo = vi.fn() as unknown as typeof Element.prototype.scrollTo;
  });

  it("renders Related and Details tabs in the right panel", () => {
    render(<ChatPage />);

    expect(screen.getByTestId("chat-rail-tab-related")).toBeDefined();
    expect(screen.getByTestId("chat-rail-tab-details")).toBeDefined();

    cleanup();
  });

  it("shows the chat input in the main pane regardless of active right-panel tab", () => {
    render(<ChatPage />);

    // Chat input is visible by default (Related tab is active).
    expect(screen.getByPlaceholderText("Type a message…")).toBeDefined();

    // Switch to Details.
    fireEvent.click(screen.getByTestId("chat-rail-tab-details"));

    // Chat input is still visible.
    expect(screen.getByPlaceholderText("Type a message…")).toBeDefined();

    cleanup();
  });

  it("reveals Conversation ID when switching to the Details tab", () => {
    render(<ChatPage />);

    // Details content is not visible on the Related tab.
    expect(screen.queryByText("c-test123")).toBeNull();

    fireEvent.click(screen.getByTestId("chat-rail-tab-details"));

    expect(screen.getByText("Conversation ID")).toBeDefined();
    expect(screen.getByText("c-test123")).toBeDefined();

    cleanup();
  });

  it("renders the three Related section headings with empty placeholders", () => {
    render(<ChatPage />);

    const headings = ["Issues", "Patches", "Documents"];
    for (const heading of headings) {
      expect(screen.getByText(heading)).toBeDefined();
    }

    expect(screen.getByText("No issues referenced by this chat yet.")).toBeDefined();
    expect(screen.getByText("No patches referenced by this chat yet.")).toBeDefined();
    expect(screen.getByText("No documents referenced by this chat yet.")).toBeDefined();

    cleanup();
  });

  it("shows a spinner while the conversation is loading", () => {
    mockIsLoading = true;
    mockConversation = undefined;
    render(<ChatPage />);

    expect(screen.getByTestId("spinner")).toBeDefined();
    expect(screen.queryByTestId("chat-rail-tab-related")).toBeNull();

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
