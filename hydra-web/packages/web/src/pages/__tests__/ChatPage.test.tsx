import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import React from "react";
import type { Conversation, SessionEvent } from "@hydra/api";

// --- Mocks ---

vi.mock("react-router-dom", () => ({
  useParams: () => ({ conversationId: "c-test123" }),
  useNavigate: () => vi.fn(),
}));

let mockConversation: Conversation | undefined;
let mockEvents: SessionEvent[] = [];
let mockIsLoading = false;
let mockError: Error | null = null;

vi.mock("../../features/chat/useConversations", () => ({
  useConversation: () => ({
    data: mockConversation,
    isLoading: mockIsLoading,
    error: mockError,
  }),
}));

vi.mock("../../features/chat/useChatTranscript", () => ({
  useChatTranscript: () => ({
    events: mockEvents,
    isLoading: false,
    error: null,
  }),
}));

vi.mock("../../features/auth/useUsername", () => ({
  useUsername: () => "alice",
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
  Avatar: ({ name, kind }: { name: string; kind?: "human" | "agent" }) => (
    <span data-testid="avatar" data-kind={kind ?? "human"} data-name={name} />
  ),
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
  Icons: new Proxy(
    {},
    {
      get: (_t, prop) => () => <span data-testid={`icon-${String(prop)}`} />,
    },
  ),
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
vi.mock("../../features/chat/ChatActivityLine.module.css", () => ({ default: cssProxy }));
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

  it("renders SessionEvent-sourced transcript with data-transcript-source=session_events", () => {
    mockEvents = [
      { type: "user_message", content: "hi from session", timestamp: "2026-04-01T10:00:00Z" },
      {
        type: "assistant_message",
        content: "hello from session",
        timestamp: "2026-04-01T10:00:30Z",
      },
    ];
    render(<ChatPage />);

    const list = screen.getByTestId("chat-message-list");
    expect(list.getAttribute("data-transcript-source")).toBe("session_events");
    expect(screen.getByText("hi from session")).toBeDefined();
    expect(screen.getByText("hello from session")).toBeDefined();

    cleanup();
  });

  // ── ChatActivityLine wiring ───────────────────────────────────────────
  // The activity line is rendered as the trailing transcript item inside
  // `ChatMessageList` and is driven by
  // `deriveActivitySteps(events, conversation.status)`. These tests verify
  // the wiring picks the right inputs and shows/hides the indicator
  // correctly without re-testing the derivation mapping table.

  it("renders the activity line inside the transcript when tail is a user_message", () => {
    mockEvents = [
      { type: "user_message", content: "hello", timestamp: "2026-04-01T10:00:00Z" },
    ];
    render(<ChatPage />);

    const indicator = screen.getByTestId("chat-activity-line");
    expect(indicator).toBeDefined();
    expect(screen.getByTestId("chat-activity-line-verb").textContent).toBe(
      "Thinking…",
    );

    // The indicator now lives INSIDE the transcript so it scrolls with it.
    const list = screen.getByTestId("chat-message-list");
    expect(list.contains(indicator)).toBe(true);

    cleanup();
  });

  it("hides the activity line when the tail event is an assistant_message", () => {
    mockEvents = [
      { type: "user_message", content: "hi", timestamp: "2026-04-01T10:00:00Z" },
      {
        type: "assistant_message",
        content: "hello",
        timestamp: "2026-04-01T10:00:30Z",
      },
    ];
    render(<ChatPage />);

    expect(screen.queryByTestId("chat-activity-line")).toBeNull();

    cleanup();
  });

  it("hides the activity line for a closed conversation with no tool steps", () => {
    mockConversation = makeConversation({ status: "closed" as Conversation["status"] });
    mockEvents = [
      { type: "user_message", content: "hi", timestamp: "2026-04-01T10:00:00Z" },
    ];
    render(<ChatPage />);

    // No historical tool steps to review → indicator stays hidden even
    // though the conversation is closed.
    expect(screen.queryByTestId("chat-activity-line")).toBeNull();

    cleanup();
  });

  it("shows a tool-use verb when a ToolUse is the trailing event", () => {
    mockEvents = [
      { type: "user_message", content: "search please", timestamp: "2026-04-01T10:00:00Z" },
      { type: "tool_use", tool_name: "Grep", payload: null, timestamp: "2026-04-01T10:00:05Z" },
    ];
    render(<ChatPage />);

    expect(screen.getByTestId("chat-activity-line-verb").textContent).toBe(
      "Searching code",
    );

    cleanup();
  });

  it("preserves chronological order of a 2-session resumption-chain transcript", () => {
    // The merge result the hook produces: first session's events, then the
    // resumed session's events. ChatPage renders them in order.
    mockEvents = [
      { type: "user_message", content: "q1", timestamp: "2026-04-01T09:01:00Z" },
      { type: "assistant_message", content: "a1", timestamp: "2026-04-01T09:02:00Z" },
      { type: "suspending", reason: "ctx", timestamp: "2026-04-01T09:30:00Z" },
      {
        type: "resumed",
        from_session_id: "t-first",
        source: "transcript",
        timestamp: "2026-04-01T10:00:30Z",
      },
      { type: "user_message", content: "q2", timestamp: "2026-04-01T10:05:00Z" },
      { type: "assistant_message", content: "a2", timestamp: "2026-04-01T10:10:00Z" },
    ];
    render(<ChatPage />);

    const list = screen.getByTestId("chat-message-list");
    expect(list.getAttribute("data-transcript-source")).toBe("session_events");

    const text = list.textContent ?? "";
    const q1 = text.indexOf("q1");
    const a1 = text.indexOf("a1");
    const q2 = text.indexOf("q2");
    const a2 = text.indexOf("a2");
    expect(q1).toBeGreaterThanOrEqual(0);
    expect(a1).toBeGreaterThan(q1);
    expect(q2).toBeGreaterThan(a1);
    expect(a2).toBeGreaterThan(q2);

    cleanup();
  });
});
