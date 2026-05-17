import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import React from "react";
import type { IssueVersionRecord } from "@hydra/api";

vi.mock("../useIssue", () => ({
  useIssue: () => ({ data: undefined }),
}));

vi.mock("../IssueLabelEditor", () => ({
  IssueLabelEditor: () => <div data-testid="label-editor" />,
}));

vi.mock("@hydra/ui", () => ({
  Badge: ({ status }: { status: string }) => (
    <span data-testid={`badge-${status}`}>{status}</span>
  ),
  TypeChip: ({ type }: { type: string }) => <span data-testid={`type-${type}`}>{type}</span>,
  Icons: {
    IconAgent: () => <span data-testid="agent-icon" />,
  },
}));

vi.mock("react-router-dom", () => ({
  Link: ({ children, to }: { children: React.ReactNode; to: string }) => (
    <a href={to}>{children}</a>
  ),
}));

vi.mock("../IssueDetailsTab.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const { IssueDetailsTab } = await import("../IssueDetailsTab");

function makeRecord(
  overrides: Partial<IssueVersionRecord["issue"]> = {},
  recordOverrides: Partial<IssueVersionRecord> = {},
): IssueVersionRecord {
  return {
    issue_id: "i-1",
    version: 1n,
    timestamp: "2026-01-02T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    labels: [],
    ...recordOverrides,
    issue: {
      type: "task",
      title: "Sample",
      description: "",
      creator: "alice",
      status: "open",
      progress: "",
      dependencies: [],
      patches: [],
      labels: [],
      ...overrides,
    },
  } as unknown as IssueVersionRecord;
}

describe("IssueDetailsTab", () => {
  beforeEach(() => vi.clearAllMocks());

  it("renders status chip, assignee, type, created, updated, and labels editor", () => {
    render(
      <IssueDetailsTab
        record={makeRecord({ assignee: "bob", type: "feature" })}
        onOpenStatusModal={() => {}}
      />,
    );
    expect(screen.getByTestId("status-chip")).toBeDefined();
    expect(screen.getByText("bob")).toBeDefined();
    expect(screen.getByText("Created")).toBeDefined();
    expect(screen.getByText("Updated")).toBeDefined();
    expect(screen.getByTestId("label-editor")).toBeDefined();
  });

  it("does not render the Parent block (parents live in the Related tab)", () => {
    render(
      <IssueDetailsTab
        record={makeRecord({
          dependencies: [{ type: "child-of", issue_id: "i-parent" }],
        })}
        onOpenStatusModal={() => {}}
      />,
    );
    expect(screen.queryByText("Parent")).toBeNull();
  });

  it("renders Repository block when session_settings.repo_name is present", () => {
    render(
      <IssueDetailsTab
        record={makeRecord({
          session_settings: { repo_name: "dourolabs/hydra", branch: "main" },
        })}
        onOpenStatusModal={() => {}}
      />,
    );
    expect(screen.getByText("Repository")).toBeDefined();
    expect(screen.getByText("dourolabs/hydra")).toBeDefined();
    expect(screen.getByText("Branch")).toBeDefined();
    expect(screen.getByText("main")).toBeDefined();
  });

  it("omits Repository and Branch blocks when not provided", () => {
    render(
      <IssueDetailsTab record={makeRecord({})} onOpenStatusModal={() => {}} />,
    );
    expect(screen.queryByText("Repository")).toBeNull();
    expect(screen.queryByText("Branch")).toBeNull();
  });

  it("shows 'Unassigned' italic when assignee is missing", () => {
    render(
      <IssueDetailsTab record={makeRecord({})} onOpenStatusModal={() => {}} />,
    );
    expect(screen.getByText("Unassigned")).toBeDefined();
  });

  it("renders Blocked on block with dep rows when blocked-on deps exist", () => {
    render(
      <IssueDetailsTab
        record={makeRecord({
          dependencies: [{ type: "blocked-on", issue_id: "i-blocker" }],
        })}
        onOpenStatusModal={() => {}}
      />,
    );
    expect(screen.getByText("Blocked on")).toBeDefined();
    expect(screen.getByText("i-blocker")).toBeDefined();
  });

  it("invokes onOpenStatusModal when the status chip is clicked", () => {
    const onOpen = vi.fn();
    render(<IssueDetailsTab record={makeRecord({})} onOpenStatusModal={onOpen} />);
    fireEvent.click(screen.getByTestId("status-chip"));
    expect(onOpen).toHaveBeenCalled();
  });
});
