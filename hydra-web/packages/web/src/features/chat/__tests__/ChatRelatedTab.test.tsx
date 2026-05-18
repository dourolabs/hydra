import { describe, it, expect, vi, beforeEach } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import React from "react";
import type {
  DocumentSummaryRecord,
  IssueSummaryRecord,
  PatchSummaryRecord,
  SessionSummaryRecord,
} from "@hydra/api";

// --- Mocks ---

const fetchNextPageIssues = vi.fn();
const fetchNextPagePatches = vi.fn();
const fetchNextPageDocuments = vi.fn();

const mockState: {
  issues: IssueSummaryRecord[];
  patches: PatchSummaryRecord[];
  documents: DocumentSummaryRecord[];
  sessionsByIssue: Map<string, SessionSummaryRecord[]>;
  isLoading: boolean;
  error: unknown;
  hasNextPage: { issues: boolean; patches: boolean; documents: boolean };
  isFetchingNextPage: { issues: boolean; patches: boolean; documents: boolean };
  fetchNextPage: {
    issues: () => void;
    patches: () => void;
    documents: () => void;
  };
} = {
  issues: [],
  patches: [],
  documents: [],
  sessionsByIssue: new Map(),
  isLoading: false,
  error: null,
  hasNextPage: { issues: false, patches: false, documents: false },
  isFetchingNextPage: { issues: false, patches: false, documents: false },
  fetchNextPage: {
    issues: fetchNextPageIssues,
    patches: fetchNextPagePatches,
    documents: fetchNextPageDocuments,
  },
};

const capturedItemRowProps: Array<{
  itemId: string;
  sessions: SessionSummaryRecord[] | undefined;
}> = [];

let lastConversationIdArg: string | null = null;

vi.mock("../useChatReferencedArtifacts", () => ({
  useChatReferencedArtifacts: (conversationId: string) => {
    lastConversationIdArg = conversationId;
    return mockState;
  },
}));

vi.mock("../../dashboard/ItemRow", () => ({
  ItemRow: ({
    item,
    sessions,
  }: {
    item: { kind: string; id: string; data: unknown };
    sessions?: SessionSummaryRecord[];
  }) => {
    capturedItemRowProps.push({ itemId: item.id, sessions });
    let title = "";
    if (item.kind === "issue") {
      title = (item.data as IssueSummaryRecord).issue.title;
    } else if (item.kind === "patch") {
      title = (item.data as PatchSummaryRecord).patch.title;
    }
    return (
      <li
        data-testid={`item-row-${item.kind}-${item.id}`}
        data-sessions-count={sessions?.length ?? 0}
      >
        {title}
      </li>
    );
  },
}));

vi.mock("@hydra/ui", () => ({
  Spinner: ({ size }: { size?: string }) => (
    <div data-testid={`spinner-${size ?? "md"}`} />
  ),
}));

vi.mock("react-router-dom", () => ({
  Link: ({ children, to }: { children: React.ReactNode; to: string }) => (
    <a href={to}>{children}</a>
  ),
}));

vi.mock("../../../components/icons/DocumentIcon", () => ({
  DocumentIcon: () => <span data-testid="document-icon" />,
}));

vi.mock("../ChatRelatedTab.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

// --- Helpers ---

function makeIssue(
  issueId: string,
  title = `Issue ${issueId}`,
): IssueSummaryRecord {
  return {
    issue_id: issueId,
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    issue: {
      type: "task",
      title,
      description: "desc",
      creator: "alice",
      status: "open",
      progress: "",
      dependencies: [],
      patches: [],
      labels: [],
    },
  };
}

function makePatch(patchId: string, title = "Test Patch"): PatchSummaryRecord {
  return {
    patch_id: patchId,
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    patch: {
      title,
      status: "Open",
      is_automatic_backup: false,
      creator: "alice",
      review_summary: { count: 0, approved: false },
      service_repo_name: "test-repo",
    },
  };
}

function makeDocument(docId: string, title = "Design Doc"): DocumentSummaryRecord {
  return {
    document_id: docId,
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    document: {
      title,
      path: `docs/${docId}.md`,
      deleted: false,
    },
  };
}

function makeSession(
  sessionId: string,
  spawnedFrom: string,
  status: "running" | "pending" | "completed" = "running",
): SessionSummaryRecord {
  return {
    session_id: sessionId,
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    session: {
      prompt: "",
      spawned_from: spawnedFrom,
      creator: "alice",
      status,
    },
  } as SessionSummaryRecord;
}

function resetState() {
  mockState.issues = [];
  mockState.patches = [];
  mockState.documents = [];
  mockState.sessionsByIssue = new Map();
  mockState.isLoading = false;
  mockState.error = null;
  mockState.hasNextPage = { issues: false, patches: false, documents: false };
  mockState.isFetchingNextPage = {
    issues: false,
    patches: false,
    documents: false,
  };
  lastConversationIdArg = null;
  capturedItemRowProps.length = 0;
  fetchNextPageIssues.mockReset();
  fetchNextPagePatches.mockReset();
  fetchNextPageDocuments.mockReset();
}

// --- Import after mocks ---
const { ChatRelatedTab } = await import("../ChatRelatedTab");

// --- Tests ---

describe("ChatRelatedTab", () => {
  beforeEach(() => {
    resetState();
    vi.clearAllMocks();
  });

  it("passes the conversationId prop through to the hook", () => {
    render(<ChatRelatedTab conversationId="c-abc" />);
    expect(lastConversationIdArg).toBe("c-abc");
  });

  it("renders the three section titles in order: Issues, Patches, Documents", () => {
    const { container } = render(<ChatRelatedTab conversationId="c-abc" />);
    const headings = Array.from(container.querySelectorAll("h3")).map(
      (h) => h.textContent?.replace(/\(\d+\)$/, "").trim(),
    );
    expect(headings).toEqual(["Issues", "Patches", "Documents"]);
  });

  it("shows empty-state copy in each section when there are no referenced artifacts", () => {
    render(<ChatRelatedTab conversationId="c-abc" />);
    expect(screen.getByText("No issues referenced by this chat yet.")).toBeDefined();
    expect(screen.getByText("No patches referenced by this chat yet.")).toBeDefined();
    expect(screen.getByText("No documents referenced by this chat yet.")).toBeDefined();
  });

  it("renders only-issues correctly", () => {
    mockState.issues = [makeIssue("i-1", "Alpha"), makeIssue("i-2", "Beta")];
    render(<ChatRelatedTab conversationId="c-abc" />);
    expect(screen.getByTestId("item-row-issue-i-1")).toBeDefined();
    expect(screen.getByTestId("item-row-issue-i-2")).toBeDefined();
    expect(screen.getByText("No patches referenced by this chat yet.")).toBeDefined();
    expect(screen.getByText("No documents referenced by this chat yet.")).toBeDefined();
  });

  it("renders only-patches correctly", () => {
    mockState.patches = [makePatch("p-1", "Fix bug")];
    render(<ChatRelatedTab conversationId="c-abc" />);
    expect(screen.getByTestId("item-row-patch-p-1")).toBeDefined();
    expect(screen.getByText("Fix bug")).toBeDefined();
    expect(screen.getByText("No issues referenced by this chat yet.")).toBeDefined();
    expect(screen.getByText("No documents referenced by this chat yet.")).toBeDefined();
  });

  it("renders only-documents correctly", () => {
    mockState.documents = [makeDocument("d-1", "Design Doc")];
    const { container } = render(<ChatRelatedTab conversationId="c-abc" />);
    expect(screen.getByText("Design Doc")).toBeDefined();
    expect(screen.getByText("docs/d-1.md")).toBeDefined();
    const link = container.querySelector('a[href="/documents/d-1"]');
    expect(link).not.toBeNull();
    expect(screen.getByText("No issues referenced by this chat yet.")).toBeDefined();
    expect(screen.getByText("No patches referenced by this chat yet.")).toBeDefined();
  });

  it("renders mixed buckets in all three sections", () => {
    mockState.issues = [makeIssue("i-1")];
    mockState.patches = [makePatch("p-1")];
    mockState.documents = [makeDocument("d-1")];
    render(<ChatRelatedTab conversationId="c-abc" />);
    expect(screen.getByTestId("item-row-issue-i-1")).toBeDefined();
    expect(screen.getByTestId("item-row-patch-p-1")).toBeDefined();
    expect(screen.getByText("Design Doc")).toBeDefined();
  });

  it("renders section counts when sections have content", () => {
    mockState.issues = [makeIssue("i-1"), makeIssue("i-2"), makeIssue("i-3")];
    const { container } = render(<ChatRelatedTab conversationId="c-abc" />);
    const issueHeading = Array.from(container.querySelectorAll("h3")).find((h) =>
      h.textContent?.startsWith("Issues"),
    );
    expect(issueHeading?.textContent).toContain("(3)");
  });

  it("shows a single spinner while loading and no sections", () => {
    mockState.isLoading = true;
    render(<ChatRelatedTab conversationId="c-abc" />);
    expect(screen.getByTestId("spinner-sm")).toBeDefined();
    expect(screen.queryByText("Issues")).toBeNull();
  });

  it("shows an error message when the hook reports an error", () => {
    mockState.error = new Error("boom");
    render(<ChatRelatedTab conversationId="c-abc" />);
    expect(screen.getByText("Failed to load referenced items.")).toBeDefined();
  });

  it("passes sessions from sessionsByIssue down to matching ItemRows", () => {
    const session = makeSession("s-1", "i-1", "running");
    mockState.issues = [makeIssue("i-1")];
    mockState.sessionsByIssue = new Map([["i-1", [session]]]);

    render(<ChatRelatedTab conversationId="c-abc" />);

    const captured = capturedItemRowProps.find((p) => p.itemId === "i-1");
    expect(captured).toBeDefined();
    expect(captured?.sessions).toEqual([session]);
    expect(
      screen
        .getByTestId("item-row-issue-i-1")
        .getAttribute("data-sessions-count"),
    ).toBe("1");
  });

  it("renders ItemRow with sessions=undefined when no map entry exists for the issue", () => {
    mockState.issues = [makeIssue("i-1"), makeIssue("i-2")];
    mockState.sessionsByIssue = new Map();

    expect(() =>
      render(<ChatRelatedTab conversationId="c-abc" />),
    ).not.toThrow();

    const i1 = capturedItemRowProps.find((p) => p.itemId === "i-1");
    const i2 = capturedItemRowProps.find((p) => p.itemId === "i-2");
    expect(i1?.sessions).toBeUndefined();
    expect(i2?.sessions).toBeUndefined();
    expect(
      screen
        .getByTestId("item-row-issue-i-1")
        .getAttribute("data-sessions-count"),
    ).toBe("0");
  });

  it("does not render Load more buttons when no section has another page", () => {
    mockState.issues = [makeIssue("i-1")];
    mockState.patches = [makePatch("p-1")];
    mockState.documents = [makeDocument("d-1")];
    render(<ChatRelatedTab conversationId="c-abc" />);
    expect(screen.queryAllByRole("button", { name: /Load more/i })).toHaveLength(0);
  });

  it("renders a Load more button in the Issues section when hasNextPage.issues is true and wires onClick", () => {
    mockState.issues = [makeIssue("i-1")];
    mockState.hasNextPage = { issues: true, patches: false, documents: false };
    render(<ChatRelatedTab conversationId="c-abc" />);

    const buttons = screen.getAllByRole("button", { name: "Load more" });
    expect(buttons).toHaveLength(1);
    fireEvent.click(buttons[0]);
    expect(fetchNextPageIssues).toHaveBeenCalledTimes(1);
    expect(fetchNextPagePatches).not.toHaveBeenCalled();
    expect(fetchNextPageDocuments).not.toHaveBeenCalled();
  });

  it("renders a Load more button in the Patches section when hasNextPage.patches is true and wires onClick", () => {
    mockState.patches = [makePatch("p-1")];
    mockState.hasNextPage = { issues: false, patches: true, documents: false };
    render(<ChatRelatedTab conversationId="c-abc" />);

    const buttons = screen.getAllByRole("button", { name: "Load more" });
    expect(buttons).toHaveLength(1);
    fireEvent.click(buttons[0]);
    expect(fetchNextPagePatches).toHaveBeenCalledTimes(1);
    expect(fetchNextPageIssues).not.toHaveBeenCalled();
    expect(fetchNextPageDocuments).not.toHaveBeenCalled();
  });

  it("renders a Load more button in the Documents section when hasNextPage.documents is true and wires onClick", () => {
    mockState.documents = [makeDocument("d-1")];
    mockState.hasNextPage = { issues: false, patches: false, documents: true };
    render(<ChatRelatedTab conversationId="c-abc" />);

    const buttons = screen.getAllByRole("button", { name: "Load more" });
    expect(buttons).toHaveLength(1);
    fireEvent.click(buttons[0]);
    expect(fetchNextPageDocuments).toHaveBeenCalledTimes(1);
    expect(fetchNextPageIssues).not.toHaveBeenCalled();
    expect(fetchNextPagePatches).not.toHaveBeenCalled();
  });

  it("shows 'Loading...' on the issues Load more button while isFetchingNextPage.issues is true and disables it", () => {
    mockState.issues = [makeIssue("i-1")];
    mockState.hasNextPage = { issues: true, patches: false, documents: false };
    mockState.isFetchingNextPage = {
      issues: true,
      patches: false,
      documents: false,
    };

    render(<ChatRelatedTab conversationId="c-abc" />);

    const button = screen.getByRole("button", { name: "Loading..." });
    expect(button).toBeDefined();
    expect((button as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(button);
    expect(fetchNextPageIssues).not.toHaveBeenCalled();
  });
});
