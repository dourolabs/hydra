import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type {
  DocumentSummaryRecord,
  IssueSummaryRecord,
  IssueVersionRecord,
  PatchSummaryRecord,
  SessionSummaryRecord,
} from "@hydra/api";

// --- API mocks ---

const mockListRelations = vi.fn();
const mockListIssues = vi.fn();
const mockListSessions = vi.fn();
const mockListConversations = vi.fn();

vi.mock("../../../api/client", () => ({
  apiClient: {
    listRelations: (...args: unknown[]) => mockListRelations(...args),
    listIssues: (...args: unknown[]) => mockListIssues(...args),
    listSessions: (...args: unknown[]) => mockListSessions(...args),
    listConversations: (...args: unknown[]) => mockListConversations(...args),
  },
}));

// --- Hook mocks ---

let mockCurrentIssue: IssueVersionRecord | undefined;
let mockPatches: PatchSummaryRecord[] = [];
let mockDocuments: DocumentSummaryRecord[] = [];

vi.mock("../useIssue", () => ({
  useIssue: () => ({ data: mockCurrentIssue, isLoading: false }),
}));

vi.mock("../../patches/useIssuePatches", () => ({
  useIssuePatches: () => ({ data: mockPatches, isLoading: false, error: null }),
}));

vi.mock("../useIssueDocuments", () => ({
  useIssueDocuments: () => ({ data: mockDocuments, isLoading: false, error: null }),
}));

// --- Component mocks ---

const capturedItemRows: Array<{
  itemId: string;
  kind: string;
  sessions: SessionSummaryRecord[] | undefined;
}> = [];

vi.mock("../../related/RailRow", () => ({
  IssueRailRow: ({
    record,
    sessions,
  }: {
    record: IssueSummaryRecord;
    sessions?: SessionSummaryRecord[];
  }) => {
    capturedItemRows.push({
      itemId: record.issue_id,
      kind: "issue",
      sessions,
    });
    return (
      <div
        data-testid={`item-row-issue-${record.issue_id}`}
        data-sessions-count={sessions?.length ?? 0}
      >
        {record.issue.title}
      </div>
    );
  },
  PatchRailRow: ({ record }: { record: PatchSummaryRecord }) => {
    capturedItemRows.push({
      itemId: record.patch_id,
      kind: "patch",
      sessions: undefined,
    });
    return (
      <div data-testid={`item-row-patch-${record.patch_id}`}>
        {record.patch.title}
      </div>
    );
  },
  DocumentRailRow: ({ record }: { record: DocumentSummaryRecord }) => (
    <div data-testid={`item-row-document-${record.document_id}`}>
      <a href={`/documents/${record.document_id}`}>
        {record.document.title ?? record.document.path ?? record.document_id}
      </a>
      {record.document.path && <span>{record.document.path}</span>}
    </div>
  ),
  ChatRailRow: ({
    conversation,
  }: {
    conversation: { conversation_id: string; title: string | null };
  }) => (
    <div data-testid={`item-row-chat-${conversation.conversation_id}`}>
      {conversation.title ?? conversation.conversation_id}
    </div>
  ),
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
  useNavigate: () => () => undefined,
}));

vi.mock("../../../components/icons/DocumentIcon", () => ({
  DocumentIcon: () => <span data-testid="document-icon" />,
}));

vi.mock("../IssueRelatedTab.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("../../related/RelatedSection.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

// --- Helpers ---

function makeIssue(
  issueId: string,
  title = `Issue ${issueId}`,
  dependencies: Array<{ type: string; issue_id: string }> = [],
): IssueSummaryRecord {
  return {
    issue_id: issueId,
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    issue: {
      type: "task",
      title,
      description: "",
      creator: "alice",
      status: "open",
      dependencies,
      patches: [],
      labels: [],
    },
  } as unknown as IssueSummaryRecord;
}

function makeIssueVersion(
  issueId: string,
  dependencies: Array<{ type: string; issue_id: string }> = [],
): IssueVersionRecord {
  return {
    issue_id: issueId,
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    labels: [],
    issue: {
      type: "task",
      title: `Issue ${issueId}`,
      description: "",
      creator: "alice",
      status: "open",
      dependencies,
      patches: [],
      labels: [],
    },
  } as unknown as IssueVersionRecord;
}

function makePatch(patchId: string, title = "Test Patch"): PatchSummaryRecord {
  return {
    patch_id: patchId,
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    patch: {
      title,
      status: "open",
      is_automatic_backup: false,
      creator: "alice",
      review_summary: { count: 0, approved: false },
      service_repo_name: "test-repo",
    },
  } as unknown as PatchSummaryRecord;
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
      archived: false,
    },
  } as unknown as DocumentSummaryRecord;
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
  } as unknown as SessionSummaryRecord;
}

function makeWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return ({ children }: { children: React.ReactNode }) =>
    React.createElement(QueryClientProvider, { client: queryClient }, children);
}

function resetState() {
  mockListRelations.mockReset();
  mockListIssues.mockReset();
  mockListSessions.mockReset();
  mockListConversations.mockReset();
  mockListConversations.mockResolvedValue({ conversations: [], next_cursor: null });
  mockCurrentIssue = undefined;
  mockPatches = [];
  mockDocuments = [];
  capturedItemRows.length = 0;
}

// --- Import after mocks ---
const { IssueRelatedTab } = await import("../IssueRelatedTab");

// --- Tests ---

describe("IssueRelatedTab", () => {
  beforeEach(() => {
    resetState();
    // Default: no relations, no related issues
    mockListRelations.mockResolvedValue({ relations: [] });
    mockListIssues.mockResolvedValue({ issues: [] });
    mockListSessions.mockResolvedValue({ sessions: [] });
    mockCurrentIssue = makeIssueVersion("i-target");
  });

  it("renders the section titles in order: Parents, Children, Patches, Conversations, Documents", async () => {
    const { container, findByText } = render(<IssueRelatedTab issueId="i-target" />, {
      wrapper: makeWrapper(),
    });
    await findByText(/No patches/);
    const headings = Array.from(container.querySelectorAll("h3")).map(
      (h) => h.textContent?.replace(/\(\d+\)$/, "").trim(),
    );
    expect(headings).toEqual([
      "Parents",
      "Children",
      "Patches",
      "Conversations",
      "Documents",
    ]);
  });

  it("shows empty-state copy in each section when there is nothing", async () => {
    render(<IssueRelatedTab issueId="i-target" />, { wrapper: makeWrapper() });
    await screen.findByText("No parent issues.");
    expect(screen.getByText("No child issues.")).toBeDefined();
    expect(screen.getByText("No patches linked to this issue.")).toBeDefined();
    expect(screen.getByText("No documents linked to this issue.")).toBeDefined();
  });

  it("renders parents from useIssue dependencies and children from relations + listIssues", async () => {
    mockCurrentIssue = makeIssueVersion("i-target", [
      { type: "child-of", issue_id: "i-parent" },
    ]);
    mockListRelations.mockResolvedValue({
      relations: [{ source_id: "i-child", target_id: "i-target", rel_type: "child-of" }],
    });
    mockListIssues.mockResolvedValue({
      issues: [makeIssue("i-parent", "Parent"), makeIssue("i-child", "Child")],
    });

    render(<IssueRelatedTab issueId="i-target" />, { wrapper: makeWrapper() });

    await screen.findByText("Parent");
    expect(screen.getByText("Child")).toBeDefined();
    expect(screen.getByTestId("item-row-issue-i-parent")).toBeDefined();
    expect(screen.getByTestId("item-row-issue-i-child")).toBeDefined();
  });

  it("passes sessions from listSessions to matching parent/child ItemRows", async () => {
    mockCurrentIssue = makeIssueVersion("i-target", [
      { type: "child-of", issue_id: "i-parent" },
    ]);
    mockListRelations.mockResolvedValue({
      relations: [{ source_id: "i-child", target_id: "i-target", rel_type: "child-of" }],
    });
    mockListIssues.mockResolvedValue({
      issues: [makeIssue("i-parent", "Parent"), makeIssue("i-child", "Child")],
    });
    mockListSessions.mockResolvedValue({
      sessions: [makeSession("s-1", "i-parent", "running")],
    });

    render(<IssueRelatedTab issueId="i-target" />, { wrapper: makeWrapper() });

    await screen.findByText("Parent");
    const parentRow = capturedItemRows.find((r) => r.itemId === "i-parent");
    expect(parentRow?.sessions?.length).toBe(1);
    const childRow = capturedItemRows.find((r) => r.itemId === "i-child");
    expect(childRow?.sessions).toBeUndefined();
  });

  it("renders patches when useIssuePatches returns data", async () => {
    mockPatches = [makePatch("p-1", "Fix bug")];
    render(<IssueRelatedTab issueId="i-target" />, { wrapper: makeWrapper() });
    await screen.findByText("Fix bug");
    expect(screen.getByTestId("item-row-patch-p-1")).toBeDefined();
  });

  it("renders documents as a link list when useIssueDocuments returns data", async () => {
    mockDocuments = [makeDocument("d-1", "Design Doc")];
    const { container } = render(<IssueRelatedTab issueId="i-target" />, {
      wrapper: makeWrapper(),
    });
    await screen.findByText("Design Doc");
    expect(screen.getByText("docs/d-1.md")).toBeDefined();
    const link = container.querySelector('a[href="/documents/d-1"]');
    expect(link).not.toBeNull();
  });

  it("renders section counts when sections have content", async () => {
    mockPatches = [makePatch("p-1"), makePatch("p-2")];
    const { container, findByTestId } = render(<IssueRelatedTab issueId="i-target" />, {
      wrapper: makeWrapper(),
    });
    await findByTestId("item-row-patch-p-1");
    const patchesHeading = Array.from(container.querySelectorAll("h3")).find((h) =>
      h.textContent?.startsWith("Patches"),
    );
    expect(patchesHeading?.textContent).toContain("(2)");
  });
});
