import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import React from "react";
import type { IssueSummaryRecord } from "@hydra/api";
import type { ChildStatus } from "../computeIssueProgress";

// --- Mocks ---

vi.mock("@hydra/ui", () => ({
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

const mockLabels = [
  {
    label_id: "lbl-1",
    name: "Feature",
    color: "#00ff00",
    recurse: false,
    hidden: false,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
  },
];

vi.mock("../../labels/useLabels", () => ({
  useLabels: () => ({ data: mockLabels }),
}));

vi.mock("../StatusBoxes", () => ({
  StatusBoxes: ({ children }: { children: ChildStatus[] }) => (
    <span data-testid="status-boxes">{children.length}</span>
  ),
}));

vi.mock("../IssueFilterSidebar.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

// --- Import after mocks ---
const { IssueFilterSidebar, LABEL_FILTER_PREFIX } = await import("../IssueFilterSidebar");

// --- Helpers ---

function makeIssue(
  id: string,
  status: string,
  labels: Array<{ label_id: string }> = [],
  assignee?: string,
): IssueSummaryRecord {
  return {
    issue_id: id,
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    issue: {
      type: "task",
      title: `Issue ${id}`,
      description: "",
      creator: "alice",
      status: status as IssueSummaryRecord["issue"]["status"],
      assignee: assignee ?? null,
      progress: "",
      dependencies: [],
      patches: [],
      labels: labels as IssueSummaryRecord["issue"]["labels"],
    },
  };
}

function renderSidebar(overrides: Partial<Parameters<typeof IssueFilterSidebar>[0]> = {}) {
  const defaultProps = {
    allIssues: [
      makeIssue("i-1", "open", [{ label_id: "lbl-1" }]),
      makeIssue("i-2", "closed", [{ label_id: "lbl-1" }]),
    ],
    activeFilter: null,
    onFilterChange: vi.fn(),
    collapsed: false,
    drawerOpen: false,
    onDrawerClose: vi.fn(),
    isActiveMap: new Map<string, boolean>(),
    username: "testuser",
    inboxCount: 3,
    myIssuesCount: 2,
  };

  const props = { ...defaultProps, ...overrides };
  const result = render(<IssueFilterSidebar {...props} />);
  return { ...result, props };
}

// --- Tests ---

describe("IssueFilterSidebar", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders Inbox, My Issues, and Everything filters", () => {
    renderSidebar();
    expect(screen.getAllByText("Inbox").length).toBeGreaterThan(0);
    expect(screen.getAllByText("My Issues").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Everything").length).toBeGreaterThan(0);
  });

  it("renders inbox count badge", () => {
    renderSidebar({ inboxCount: 5 });
    expect(screen.getAllByText("5").length).toBeGreaterThan(0);
  });

  it("renders my issues count badge", () => {
    renderSidebar({ myIssuesCount: 7 });
    expect(screen.getAllByText("7").length).toBeGreaterThan(0);
  });

  it("does not render inbox count when zero", () => {
    renderSidebar({ inboxCount: 0 });
    // Inbox label present, but no "0" badge
    expect(screen.getAllByText("Inbox").length).toBeGreaterThan(0);
    expect(screen.queryByText("0")).toBeNull();
  });

  it("calls onFilterChange with 'inbox' on Inbox click", () => {
    const { props } = renderSidebar();
    // There are duplicate instances (desktop + mobile) — click the first
    const inboxElements = screen.getAllByText("Inbox");
    fireEvent.click(inboxElements[0].closest("li")!);
    expect(props.onFilterChange).toHaveBeenCalledWith("inbox");
  });

  it("calls onFilterChange with 'my-issues' on My Issues click", () => {
    const { props } = renderSidebar();
    const myIssuesElements = screen.getAllByText("My Issues");
    fireEvent.click(myIssuesElements[0].closest("li")!);
    expect(props.onFilterChange).toHaveBeenCalledWith("my-issues");
  });

  it("calls onFilterChange with null on Everything click", () => {
    const { props } = renderSidebar();
    const everythingElements = screen.getAllByText("Everything");
    fireEvent.click(everythingElements[0].closest("li")!);
    expect(props.onFilterChange).toHaveBeenCalledWith(null);
  });

  it("closes drawer on filter selection", () => {
    const { props } = renderSidebar({ drawerOpen: true });
    const inboxElements = screen.getAllByText("Inbox");
    fireEvent.click(inboxElements[0].closest("li")!);
    expect(props.onDrawerClose).toHaveBeenCalled();
  });

  it("marks active filter with active class", () => {
    renderSidebar({ activeFilter: "inbox" });
    const inboxItems = screen.getAllByText("Inbox");
    const li = inboxItems[0].closest("li")!;
    expect(li.className).toContain("active");
  });

  it("renders label filters from useLabels data", () => {
    renderSidebar();
    expect(screen.getAllByText("Feature").length).toBeGreaterThan(0);
  });

  it("shows label progress (closed/total)", () => {
    renderSidebar();
    // 1 closed out of 2 issues with label lbl-1
    expect(screen.getAllByText("1/2").length).toBeGreaterThan(0);
  });

  it("calls onFilterChange with label filter on label click", () => {
    const { props } = renderSidebar();
    const featureElements = screen.getAllByText("Feature");
    fireEvent.click(featureElements[0].closest("li")!);
    expect(props.onFilterChange).toHaveBeenCalledWith(`${LABEL_FILTER_PREFIX}lbl-1`);
  });

  it("deselects label filter when clicking the already-active label", () => {
    const { props } = renderSidebar({ activeFilter: `${LABEL_FILTER_PREFIX}lbl-1` });
    const featureElements = screen.getAllByText("Feature");
    fireEvent.click(featureElements[0].closest("li")!);
    expect(props.onFilterChange).toHaveBeenCalledWith(null);
  });

  it("supports keyboard Enter to select filter", () => {
    const { props } = renderSidebar();
    const inboxElements = screen.getAllByText("Inbox");
    const li = inboxElements[0].closest("li")!;
    fireEvent.keyDown(li, { key: "Enter" });
    expect(props.onFilterChange).toHaveBeenCalledWith("inbox");
  });

  it("supports keyboard Space to select filter", () => {
    const { props } = renderSidebar();
    const inboxElements = screen.getAllByText("Inbox");
    const li = inboxElements[0].closest("li")!;
    fireEvent.keyDown(li, { key: " " });
    expect(props.onFilterChange).toHaveBeenCalledWith("inbox");
  });

  it("renders backdrop when drawer is open", () => {
    const { container } = renderSidebar({ drawerOpen: true });
    const backdrop = container.querySelector(".backdrop");
    expect(backdrop).not.toBeNull();
  });

  it("calls onDrawerClose when backdrop is clicked", () => {
    const { container, props } = renderSidebar({ drawerOpen: true });
    const backdrop = container.querySelector(".backdrop")!;
    fireEvent.click(backdrop);
    expect(props.onDrawerClose).toHaveBeenCalled();
  });

  it("exports LABEL_FILTER_PREFIX constant", () => {
    expect(LABEL_FILTER_PREFIX).toBe("label:");
  });
});
