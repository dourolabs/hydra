// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import React from "react";

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

vi.mock("../IssueFilterSidebar.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

// --- Import after mocks ---
const { IssueFilterSidebar } = await import("../IssueFilterSidebar");

// --- Helpers ---

function renderSidebar(overrides: Partial<Parameters<typeof IssueFilterSidebar>[0]> = {}) {
  const defaultProps = {
    activeFilter: "your-issues" as string | null,
    onFilterChange: vi.fn(),
    collapsed: false,
    drawerOpen: false,
    onDrawerClose: vi.fn(),
    username: "testuser",
    yourIssuesCount: 3,
    assignedCount: 2,
  };

  const props = { ...defaultProps, ...overrides };
  const result = render(<IssueFilterSidebar {...props} />);
  return { ...result, props };
}

// --- Tests ---

describe("IssueFilterSidebar", () => {
  afterEach(() => {
    cleanup();
  });

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders Your Issues and Assigned to You filters", () => {
    renderSidebar();
    expect(screen.getAllByText("Your Issues").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Assigned to You").length).toBeGreaterThan(0);
  });

  it("does not render Inbox, My Issues, Everything, or Labels", () => {
    renderSidebar();
    expect(screen.queryByText("Inbox")).toBeNull();
    expect(screen.queryByText("My Issues")).toBeNull();
    expect(screen.queryByText("Everything")).toBeNull();
    expect(screen.queryByText("Labels")).toBeNull();
  });

  it("renders your issues count badge", () => {
    renderSidebar({ yourIssuesCount: 5 });
    expect(screen.getAllByText("5").length).toBeGreaterThan(0);
  });

  it("renders assigned count badge", () => {
    renderSidebar({ assignedCount: 7 });
    expect(screen.getAllByText("7").length).toBeGreaterThan(0);
  });

  it("does not render your issues count when zero", () => {
    renderSidebar({ yourIssuesCount: 0 });
    expect(screen.getAllByText("Your Issues").length).toBeGreaterThan(0);
    expect(screen.queryByText("0")).toBeNull();
  });

  it("does not render assigned count when zero", () => {
    renderSidebar({ assignedCount: 0 });
    expect(screen.getAllByText("Assigned to You").length).toBeGreaterThan(0);
    expect(screen.queryByText("0")).toBeNull();
  });

  it("calls onFilterChange with 'your-issues' on Your Issues click", () => {
    const { props } = renderSidebar();
    const elements = screen.getAllByText("Your Issues");
    fireEvent.click(elements[0].closest("li")!);
    expect(props.onFilterChange).toHaveBeenCalledWith("your-issues");
  });

  it("calls onFilterChange with 'assigned' on Assigned to You click", () => {
    const { props } = renderSidebar();
    const elements = screen.getAllByText("Assigned to You");
    fireEvent.click(elements[0].closest("li")!);
    expect(props.onFilterChange).toHaveBeenCalledWith("assigned");
  });

  it("closes drawer on filter selection", () => {
    const { props } = renderSidebar({ drawerOpen: true });
    const elements = screen.getAllByText("Your Issues");
    fireEvent.click(elements[0].closest("li")!);
    expect(props.onDrawerClose).toHaveBeenCalled();
  });

  it("marks active filter with active class", () => {
    renderSidebar({ activeFilter: "your-issues" });
    const items = screen.getAllByText("Your Issues");
    const li = items[0].closest("li")!;
    expect(li.className).toContain("active");
  });

  it("marks assigned filter with active class", () => {
    renderSidebar({ activeFilter: "assigned" });
    const items = screen.getAllByText("Assigned to You");
    const li = items[0].closest("li")!;
    expect(li.className).toContain("active");
  });

  it("supports keyboard Enter to select filter", () => {
    const { props } = renderSidebar();
    const elements = screen.getAllByText("Your Issues");
    const li = elements[0].closest("li")!;
    fireEvent.keyDown(li, { key: "Enter" });
    expect(props.onFilterChange).toHaveBeenCalledWith("your-issues");
  });

  it("supports keyboard Space to select filter", () => {
    const { props } = renderSidebar();
    const elements = screen.getAllByText("Assigned to You");
    const li = elements[0].closest("li")!;
    fireEvent.keyDown(li, { key: " " });
    expect(props.onFilterChange).toHaveBeenCalledWith("assigned");
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
});
