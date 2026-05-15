import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import React from "react";
import type {
  DocumentSummaryRecord,
  IssueSummaryRecord,
  PatchSummaryRecord,
} from "@hydra/api";

// --- Mocks ---

const mockState: {
  issues: IssueSummaryRecord[];
  patches: PatchSummaryRecord[];
  documents: DocumentSummaryRecord[];
  isLoading: boolean;
  error: unknown;
} = {
  issues: [],
  patches: [],
  documents: [],
  isLoading: false,
  error: null,
};

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
  }: {
    item: { kind: string; id: string; data: unknown };
  }) => {
    let title = "";
    if (item.kind === "issue") {
      title = (item.data as IssueSummaryRecord).issue.title;
    } else if (item.kind === "patch") {
      title = (item.data as PatchSummaryRecord).patch.title;
    }
    return (
      <li data-testid={`item-row-${item.kind}-${item.id}`}>{title}</li>
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

function resetState() {
  mockState.issues = [];
  mockState.patches = [];
  mockState.documents = [];
  mockState.isLoading = false;
  mockState.error = null;
  lastConversationIdArg = null;
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
    // Sections aren't rendered during loading
    expect(screen.queryByText("Issues")).toBeNull();
  });

  it("shows an error message when the hook reports an error", () => {
    mockState.error = new Error("boom");
    render(<ChatRelatedTab conversationId="c-abc" />);
    expect(screen.getByText("Failed to load referenced items.")).toBeDefined();
  });
});
