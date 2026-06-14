import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { useState } from "react";
import type { IssueVersionRecord } from "@hydra/api";

vi.mock("../IssueRelatedTab", () => ({
  IssueRelatedTab: ({ issueId }: { issueId: string }) => (
    <div data-testid="related-tab-stub">related:{issueId}</div>
  ),
}));

vi.mock("../IssueActivity", () => ({
  IssueActivity: ({ issueId }: { issueId: string }) => (
    <div data-testid="activity-tab-stub">activity:{issueId}</div>
  ),
}));

vi.mock("../IssueDetailsTab", () => ({
  IssueDetailsTab: ({
    record,
    onOpenStatusModal,
  }: {
    record: IssueVersionRecord;
    onOpenStatusModal: () => void;
  }) => (
    <div data-testid="details-tab-stub">
      details:{record.issue_id}
      <button onClick={onOpenStatusModal} data-testid="open-status">open</button>
    </div>
  ),
}));

vi.mock("../IssueRightPanel.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const { IssueRightPanel } = await import("../IssueRightPanel");

function makeRecord(): IssueVersionRecord {
  return {
    issue_id: "i-1",
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    labels: [],
    issue: {
      type: "task",
      title: "Sample",
      description: "",
      creator: "alice",
      status: "open",
      dependencies: [],
      patches: [],
      labels: [],
    },
  } as unknown as IssueVersionRecord;
}

describe("IssueRightPanel", () => {
  beforeEach(() => vi.clearAllMocks());

  it("renders the three tab buttons with the right testids", () => {
    render(<IssueRightPanel record={makeRecord()} onOpenStatusModal={() => {}} />);
    expect(screen.getByTestId("issue-rail-tab-related")).toBeDefined();
    expect(screen.getByTestId("issue-rail-tab-activity")).toBeDefined();
    expect(screen.getByTestId("issue-rail-tab-details")).toBeDefined();
  });

  it("defaults to the Related tab when uncontrolled", () => {
    render(<IssueRightPanel record={makeRecord()} onOpenStatusModal={() => {}} />);
    expect(screen.getByTestId("related-tab-stub")).toBeDefined();
    expect(screen.queryByTestId("activity-tab-stub")).toBeNull();
    expect(screen.queryByTestId("details-tab-stub")).toBeNull();
  });

  it("switches tab in uncontrolled mode on click", () => {
    render(<IssueRightPanel record={makeRecord()} onOpenStatusModal={() => {}} />);
    fireEvent.click(screen.getByTestId("issue-rail-tab-activity"));
    expect(screen.getByTestId("activity-tab-stub")).toBeDefined();
    fireEvent.click(screen.getByTestId("issue-rail-tab-details"));
    expect(screen.getByTestId("details-tab-stub")).toBeDefined();
  });

  it("aria-selected reflects active tab", () => {
    render(<IssueRightPanel record={makeRecord()} onOpenStatusModal={() => {}} />);
    expect(
      screen.getByTestId("issue-rail-tab-related").getAttribute("aria-selected"),
    ).toBe("true");
    fireEvent.click(screen.getByTestId("issue-rail-tab-details"));
    expect(
      screen.getByTestId("issue-rail-tab-details").getAttribute("aria-selected"),
    ).toBe("true");
    expect(
      screen.getByTestId("issue-rail-tab-related").getAttribute("aria-selected"),
    ).toBe("false");
  });

  it("respects controlled activeTabKey prop and calls onTabChange", () => {
    function Controlled() {
      const [tab, setTab] = useState<"related" | "activity" | "details">("activity");
      return (
        <IssueRightPanel
          record={makeRecord()}
          onOpenStatusModal={() => {}}
          activeTabKey={tab}
          onTabChange={setTab}
        />
      );
    }
    render(<Controlled />);
    expect(screen.getByTestId("activity-tab-stub")).toBeDefined();
    fireEvent.click(screen.getByTestId("issue-rail-tab-details"));
    expect(screen.getByTestId("details-tab-stub")).toBeDefined();
  });

  it("invokes onTabChange when uncontrolled too (so parent can mirror the state)", () => {
    const onTabChange = vi.fn();
    render(
      <IssueRightPanel
        record={makeRecord()}
        onOpenStatusModal={() => {}}
        onTabChange={onTabChange}
      />,
    );
    fireEvent.click(screen.getByTestId("issue-rail-tab-activity"));
    expect(onTabChange).toHaveBeenCalledWith("activity");
  });

  it("forwards data-mobile-active to the wrapper", () => {
    const { container } = render(
      <IssueRightPanel
        record={makeRecord()}
        onOpenStatusModal={() => {}}
        data-mobile-active="true"
      />,
    );
    const aside = container.querySelector("aside");
    expect(aside?.getAttribute("data-mobile-active")).toBe("true");
  });

  it("forwards onOpenStatusModal to the Details tab", () => {
    const onOpenStatusModal = vi.fn();
    render(
      <IssueRightPanel
        record={makeRecord()}
        onOpenStatusModal={onOpenStatusModal}
        activeTabKey="details"
      />,
    );
    fireEvent.click(screen.getByTestId("open-status"));
    expect(onOpenStatusModal).toHaveBeenCalled();
  });
});
