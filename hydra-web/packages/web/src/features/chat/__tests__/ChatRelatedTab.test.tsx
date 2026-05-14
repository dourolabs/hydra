import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import React from "react";
import type {
  DocumentSummaryRecord,
  IssueSummaryRecord,
  PatchSummaryRecord,
  SessionSummaryRecord,
} from "@hydra/api";

// --- Mocks ---

// Track exclude IDs that the component passes into each issue hook,
// so we can assert the dedup behavior.
let lastAttentionExclude: Set<string> | null = null;
let lastTopLevelExclude: Set<string> | null = null;

const mockState: {
  active: {
    issues: IssueSummaryRecord[];
    sessionsByIssue: Map<string, SessionSummaryRecord[]>;
    isLoading: boolean;
  };
  attentionFixture: IssueSummaryRecord[];
  attentionLoading: boolean;
  topLevelFixture: IssueSummaryRecord[];
  topLevelLoading: boolean;
  documents: DocumentSummaryRecord[];
  documentsLoading: boolean;
  patches: PatchSummaryRecord[];
  patchesLoading: boolean;
} = {
  active: { issues: [], sessionsByIssue: new Map(), isLoading: false },
  attentionFixture: [],
  attentionLoading: false,
  topLevelFixture: [],
  topLevelLoading: false,
  documents: [],
  documentsLoading: false,
  patches: [],
  patchesLoading: false,
};

vi.mock("../useChatActiveSessionIssues", () => ({
  useChatActiveSessionIssues: () => mockState.active,
}));

// Mirror the real hook's dedup behavior so the test exercises it:
// it accepts an excludeIds set and filters its fixture.
vi.mock("../useChatAttentionIssues", () => ({
  useChatAttentionIssues: (excludeIds: Set<string>) => {
    lastAttentionExclude = excludeIds;
    return {
      issues: mockState.attentionFixture.filter((i) => !excludeIds.has(i.issue_id)),
      isLoading: mockState.attentionLoading,
    };
  },
}));

vi.mock("../useChatTopLevelIssues", () => ({
  useChatTopLevelIssues: (excludeIds: Set<string>) => {
    lastTopLevelExclude = excludeIds;
    // Top-level fixture is pre-filtered to no-child-of in the real hook,
    // so we just filter exclude.
    return {
      issues: mockState.topLevelFixture.filter((i) => !excludeIds.has(i.issue_id)),
      isLoading: mockState.topLevelLoading,
    };
  },
}));

vi.mock("../useChatRelatedDocuments", () => ({
  useChatRelatedDocuments: () => ({
    documents: mockState.documents,
    isLoading: mockState.documentsLoading,
  }),
}));

vi.mock("../useChatRelatedPatches", () => ({
  useChatRelatedPatches: () => ({
    patches: mockState.patches,
    isLoading: mockState.patchesLoading,
  }),
}));

vi.mock("../../dashboard/ItemRow", () => ({
  ItemRow: ({
    item,
    isActive,
  }: {
    item: { kind: string; id: string; data: unknown };
    isActive?: boolean;
  }) => {
    let title = "";
    if (item.kind === "issue") {
      title = (item.data as IssueSummaryRecord).issue.title;
    } else if (item.kind === "patch") {
      title = (item.data as PatchSummaryRecord).patch.title;
    }
    return (
      <li
        data-testid={`item-row-${item.kind}-${item.id}`}
        data-active={isActive ? "true" : "false"}
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
  overrides: { title?: string; assignee?: string; dependencies?: IssueSummaryRecord["issue"]["dependencies"]; status?: IssueSummaryRecord["issue"]["status"] } = {},
): IssueSummaryRecord {
  return {
    issue_id: issueId,
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    issue: {
      type: "task",
      title: overrides.title ?? `Issue ${issueId}`,
      description: "desc",
      creator: "alice",
      status: overrides.status ?? "open",
      assignee: overrides.assignee,
      progress: "",
      dependencies: overrides.dependencies ?? [],
      patches: [],
      labels: [],
    },
  };
}

function makeSession(issueId: string): SessionSummaryRecord {
  return {
    session_id: `s-${issueId}`,
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    session: {
      prompt: "do work",
      spawned_from: issueId,
      creator: "alice",
      status: "running",
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
      labels: [],
    },
  };
}

function resetState() {
  mockState.active = { issues: [], sessionsByIssue: new Map(), isLoading: false };
  mockState.attentionFixture = [];
  mockState.attentionLoading = false;
  mockState.topLevelFixture = [];
  mockState.topLevelLoading = false;
  mockState.documents = [];
  mockState.documentsLoading = false;
  mockState.patches = [];
  mockState.patchesLoading = false;
  lastAttentionExclude = null;
  lastTopLevelExclude = null;
}

// --- Import after mocks ---
const { ChatRelatedTab } = await import("../ChatRelatedTab");

// --- Tests ---

describe("ChatRelatedTab", () => {
  beforeEach(() => {
    resetState();
    vi.clearAllMocks();
  });

  it("renders all 5 section titles in order", () => {
    const { container } = render(<ChatRelatedTab />);
    const headings = Array.from(container.querySelectorAll("h3")).map(
      (h) => h.textContent?.replace(/\(\d+\)$/, "").trim(),
    );
    expect(headings).toEqual([
      "Issues with active sessions",
      "Needs my attention",
      "Top-level issues",
      "Documents",
      "Patches",
    ]);
  });

  it("shows '(empty)' placeholders when all fixtures are empty", () => {
    render(<ChatRelatedTab />);
    expect(screen.getAllByText("(empty)")).toHaveLength(5);
  });

  it("renders active-session issues with isActive=true", () => {
    mockState.active = {
      issues: [makeIssue("i-active", { title: "Active 1" })],
      sessionsByIssue: new Map([["i-active", [makeSession("i-active")]]]),
      isLoading: false,
    };
    render(<ChatRelatedTab />);
    const row = screen.getByTestId("item-row-issue-i-active");
    expect(row.getAttribute("data-active")).toBe("true");
    expect(row.textContent).toBe("Active 1");
  });

  it("excludes active-session issue ids from attention and top-level hooks", () => {
    mockState.active = {
      issues: [makeIssue("i-active", { title: "Active 1" })],
      sessionsByIssue: new Map([["i-active", [makeSession("i-active")]]]),
      isLoading: false,
    };
    // Same issue id is also in attention/top-level fixtures
    mockState.attentionFixture = [makeIssue("i-active", { title: "Active 1" })];
    mockState.topLevelFixture = [makeIssue("i-active", { title: "Active 1" })];

    render(<ChatRelatedTab />);
    expect(lastAttentionExclude?.has("i-active")).toBe(true);
    expect(lastTopLevelExclude?.has("i-active")).toBe(true);

    // Issue should appear only once in the DOM (under active sessions)
    const matches = screen.getAllByText("Active 1");
    expect(matches.length).toBe(1);
  });

  it("attention issue is excluded from top-level via excludeIds", () => {
    mockState.attentionFixture = [makeIssue("i-needs-me", { title: "Needs me" })];
    mockState.topLevelFixture = [makeIssue("i-needs-me", { title: "Needs me" })];

    render(<ChatRelatedTab />);
    // Attention hook receives only the active ids; top-level receives active + attention
    expect(lastAttentionExclude?.has("i-needs-me")).toBe(false);
    expect(lastTopLevelExclude?.has("i-needs-me")).toBe(true);

    // Should appear only once (in attention) in the rendered output
    const matches = screen.getAllByText("Needs me");
    expect(matches.length).toBe(1);
  });

  it("renders documents with title, path, and a link", () => {
    mockState.documents = [makeDocument("d-1", "Design Doc")];
    const { container } = render(<ChatRelatedTab />);
    expect(screen.getByText("Design Doc")).toBeDefined();
    expect(screen.getByText("docs/d-1.md")).toBeDefined();
    const link = container.querySelector('a[href="/documents/d-1"]');
    expect(link).not.toBeNull();
  });

  it("renders patches via ItemRow", () => {
    mockState.patches = [makePatch("p-1", "First Patch"), makePatch("p-2", "Second Patch")];
    render(<ChatRelatedTab />);
    expect(screen.getByTestId("item-row-patch-p-1")).toBeDefined();
    expect(screen.getByTestId("item-row-patch-p-2")).toBeDefined();
    expect(screen.getByText("First Patch")).toBeDefined();
  });

  it("renders section counts when sections have content", () => {
    mockState.documents = [makeDocument("d-1"), makeDocument("d-2"), makeDocument("d-3")];
    const { container } = render(<ChatRelatedTab />);
    const docHeading = Array.from(container.querySelectorAll("h3")).find((h) =>
      h.textContent?.startsWith("Documents"),
    );
    expect(docHeading?.textContent).toContain("(3)");
  });

  it("shows spinner while a section is loading", () => {
    mockState.documentsLoading = true;
    render(<ChatRelatedTab />);
    expect(screen.getByTestId("spinner-sm")).toBeDefined();
  });
});
