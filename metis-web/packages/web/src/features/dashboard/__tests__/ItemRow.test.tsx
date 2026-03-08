import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import React from "react";
import type { WorkItem } from "../useTransitiveWorkItems";
import type { JobSummaryRecord } from "@metis/api";
import type { ChildStatus } from "../computeIssueProgress";

// --- Mocks ---

const mockNavigate = vi.fn();
vi.mock("react-router-dom", () => ({
  useNavigate: () => mockNavigate,
}));

const mockMutate = vi.fn();
vi.mock("@tanstack/react-query", () => ({
  useMutation: () => ({ mutate: mockMutate, isPending: false }),
  useQueryClient: () => ({
    cancelQueries: vi.fn(),
    getQueriesData: vi.fn(() => []),
    setQueriesData: vi.fn(),
    setQueryData: vi.fn(),
    invalidateQueries: vi.fn(),
  }),
}));

vi.mock("../../auth/useAuth", () => ({
  useAuth: () => ({
    user: { actor: { type: "user", username: "testuser" } },
  }),
}));

vi.mock("../../../api/client", () => ({
  apiClient: { removeLabelFromObject: vi.fn() },
}));

vi.mock("@metis/ui", () => ({
  Avatar: ({ name }: { name: string }) => <span data-testid="avatar">{name}</span>,
  Badge: ({ status }: { status: string }) => <span data-testid="badge">{status}</span>,
  useKeyboardClick: (handler: () => void) => ({
    tabIndex: 0,
    role: "button",
    onKeyDown: (e: React.KeyboardEvent) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        handler();
      }
    },
  }),
}));

vi.mock("../useJobDuration", () => ({
  useJobDuration: (jobs: JobSummaryRecord[] | undefined) => {
    const running = jobs?.find(
      (j) => j.task.status === "running" || j.task.status === "pending",
    );
    if (running) return { durationText: "0:05", isRunning: true };
    return { durationText: "\u2014", isRunning: false };
  },
}));

vi.mock("../useSwipeToArchive", () => ({
  useSwipeToArchive: vi.fn(),
}));

vi.mock("../StatusBoxes", () => ({
  StatusBoxes: ({ children }: { children: ChildStatus[] }) => (
    <span data-testid="status-boxes">{children.length}</span>
  ),
}));

vi.mock("../../labels/LabelChip", () => ({
  LabelChip: ({ name }: { name: string }) => <span data-testid="label-chip">{name}</span>,
}));

vi.mock("../ItemRow.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("../../../utils/statusMapping", () => ({
  normalizeIssueStatus: (s: string) => s,
  normalizePatchStatus: (s: string) => s.toLowerCase(),
}));

vi.mock("../../../utils/text", () => ({
  descriptionSnippet: (s: string) => s?.slice(0, 50) ?? "",
}));

// --- Import after mocks ---
const { ItemRow } = await import("../ItemRow");

// --- Helpers ---

function makeIssueItem(overrides: Partial<WorkItem & { kind: "issue" }> = {}): WorkItem {
  return {
    kind: "issue",
    id: "i-test1",
    lastUpdated: "2026-01-01T00:00:00Z",
    isTerminal: false,
    data: {
      issue_id: "i-test1",
      version: 1n,
      timestamp: "2026-01-01T00:00:00Z",
      creation_time: "2026-01-01T00:00:00Z",
      issue: {
        type: "task",
        title: "Test Issue",
        description: "A test issue",
        creator: "alice",
        status: "open",
        progress: "",
        dependencies: [],
        patches: [],
        labels: [],
      },
    },
    ...overrides,
  } as WorkItem;
}

function makePatchItem(): WorkItem {
  return {
    kind: "patch",
    id: "p-test1",
    lastUpdated: "2026-01-01T00:00:00Z",
    isTerminal: false,
    sourceIssueId: "i-test1",
    data: {
      patch_id: "p-test1",
      version: 1n,
      timestamp: "2026-01-01T00:00:00Z",
      creation_time: "2026-01-01T00:00:00Z",
      patch: {
        title: "Test Patch",
        status: "Open",
        is_automatic_backup: false,
        creator: "alice",
        review_summary: { count: 0, approved: false },
        service_repo_name: "test-repo",
      },
    },
  } as WorkItem;
}

function makeRunningJob(): JobSummaryRecord {
  return {
    job_id: "t-job1",
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    task: {
      prompt: "do work",
      creator: "alice",
      status: "running",
      start_time: new Date(Date.now() - 5000).toISOString(),
    },
  };
}

function makeDocumentItem(): WorkItem {
  return {
    kind: "document",
    id: "d-test1",
    lastUpdated: "2026-01-01T00:00:00Z",
    isTerminal: false,
    sourceIssueId: "i-test1",
    data: {
      document_id: "d-test1",
      version: 1n,
      timestamp: "2026-01-01T00:00:00Z",
      creation_time: "2026-01-01T00:00:00Z",
      document: {
        title: "Design Doc",
        path: "designs/my-doc.md",
        deleted: false,
        labels: [],
      },
    },
  } as WorkItem;
}

// --- Tests ---

describe("ItemRow", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders issue title", () => {
    render(<ItemRow item={makeIssueItem()} />);
    expect(screen.getByText("Test Issue")).toBeDefined();
  });

  it("renders description snippet when title is empty", () => {
    const item = makeIssueItem();
    if (item.kind === "issue") {
      item.data.issue.title = "";
      item.data.issue.description = "Description text here";
    }
    render(<ItemRow item={item} />);
    expect(screen.getByText("Description text here")).toBeDefined();
  });

  it("renders patch title and badge", () => {
    render(<ItemRow item={makePatchItem()} />);
    expect(screen.getByText("Test Patch")).toBeDefined();
    expect(screen.getByTestId("badge")).toBeDefined();
  });

  it("navigates to issue detail on click", () => {
    const { container } = render(<ItemRow item={makeIssueItem()} />);
    const li = container.querySelector("li")!;
    fireEvent.click(li);
    expect(mockNavigate).toHaveBeenCalledWith(
      expect.stringContaining("/issues/i-test1"),
    );
  });

  it("navigates on Enter key", () => {
    const { container } = render(<ItemRow item={makeIssueItem()} />);
    const li = container.querySelector("li")!;
    fireEvent.keyDown(li, { key: "Enter" });
    expect(mockNavigate).toHaveBeenCalled();
  });

  it("shows running job duration text", () => {
    render(<ItemRow item={makeIssueItem()} jobs={[makeRunningJob()]} />);
    expect(screen.getByText("0:05")).toBeDefined();
  });

  it("shows dash when no jobs", () => {
    render(<ItemRow item={makeIssueItem()} />);
    expect(screen.getByText("\u2014")).toBeDefined();
  });

  it("shows archive button when inboxLabelId is set for issues", () => {
    render(<ItemRow item={makeIssueItem()} inboxLabelId="lbl-inbox" />);
    const archiveBtn = screen.getByTitle("Archive");
    expect(archiveBtn).toBeDefined();
  });

  it("does not show archive button for patches", () => {
    render(<ItemRow item={makePatchItem()} inboxLabelId="lbl-inbox" />);
    expect(screen.queryByTitle("Archive")).toBeNull();
  });

  it("calls archive mutation on archive button click", () => {
    render(<ItemRow item={makeIssueItem()} inboxLabelId="lbl-inbox" />);
    const archiveBtn = screen.getByTitle("Archive");
    fireEvent.click(archiveBtn);
    expect(mockMutate).toHaveBeenCalledWith("i-test1");
  });

  it("shows assignee avatar", () => {
    const item = makeIssueItem();
    if (item.kind === "issue") {
      item.data.issue.assignee = "bob";
    }
    render(<ItemRow item={item} />);
    expect(screen.getByTestId("avatar")).toBeDefined();
    expect(screen.getByText("bob")).toBeDefined();
  });

  it("renders child status boxes", () => {
    const childStatuses: ChildStatus[] = [
      { id: "c1", status: "open", hasActiveTask: false, assignedToUser: false },
      { id: "c2", status: "closed", hasActiveTask: false, assignedToUser: false },
    ];
    render(<ItemRow item={makeIssueItem()} childStatuses={childStatuses} />);
    expect(screen.getByTestId("status-boxes")).toBeDefined();
    expect(screen.getByText("2")).toBeDefined();
  });

  it("renders labels for issues", () => {
    const item = makeIssueItem();
    if (item.kind === "issue") {
      item.data.issue.labels = [
        { label_id: "l1", name: "bug", color: "#ff0000", recurse: false, hidden: false },
      ];
    }
    render(<ItemRow item={item} />);
    expect(screen.getByTestId("label-chip")).toBeDefined();
    expect(screen.getByText("bug")).toBeDefined();
  });

  it("filters out hidden labels", () => {
    const item = makeIssueItem();
    if (item.kind === "issue") {
      item.data.issue.labels = [
        { label_id: "l1", name: "visible", color: "#ff0000", recurse: false, hidden: false },
        { label_id: "l2", name: "hidden-one", color: "#00ff00", recurse: false, hidden: true },
      ];
    }
    render(<ItemRow item={item} />);
    expect(screen.getByText("visible")).toBeDefined();
    expect(screen.queryByText("hidden-one")).toBeNull();
  });

  it("applies terminal class when item is terminal", () => {
    const item = makeIssueItem({ isTerminal: true });
    const { container } = render(<ItemRow item={item} />);
    const li = container.querySelector("li")!;
    expect(li.className).toContain("terminal");
  });

  it("shows progress line for issues with progress", () => {
    const item = makeIssueItem();
    if (item.kind === "issue") {
      item.data.issue.progress = "50% complete";
    }
    render(<ItemRow item={item} />);
    expect(screen.getByText("50% complete")).toBeDefined();
  });

  it("renders document title and navigates to document path", () => {
    render(<ItemRow item={makeDocumentItem()} />);
    expect(screen.getByText("Design Doc")).toBeDefined();
    const { container } = render(<ItemRow item={makeDocumentItem()} />);
    const li = container.querySelector("li")!;
    fireEvent.click(li);
    expect(mockNavigate).toHaveBeenCalledWith(
      expect.stringContaining("/documents/d-test1"),
    );
  });
});
