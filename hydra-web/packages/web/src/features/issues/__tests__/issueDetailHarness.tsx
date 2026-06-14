import { vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type { Principal } from "@hydra/api";

// vi.mock calls are hoisted to the top of this module by Vitest's transformer.
// When a test file imports this harness, the hoisted mocks register before
// the test file's own `await import("../IssueDetail")` resolves, so IssueDetail
// and its dependencies pick up these stubs.

vi.mock("../useIssue", () => ({
  useIssue: () => ({ data: undefined }),
}));

vi.mock("../../sessions/useSessionsByIssue", () => ({
  useSessionsByIssue: () => ({ data: [] }),
}));

vi.mock("../../dashboard/useSessionDuration", () => ({
  useSessionDuration: () => ({ durationText: "", isRunning: false }),
}));

vi.mock("../IssueRightPanel", () => ({
  IssueRightPanel: () => <div data-testid="right-panel-stub" />,
}));

vi.mock("../IssueUpdateModal", () => ({
  IssueUpdateModal: () => null,
}));

vi.mock("../CommentsPanel", () => ({
  CommentsPanel: ({ issueId }: { issueId: string }) => (
    <div data-testid="comments-panel-stub" data-issue-id={issueId} />
  ),
}));

vi.mock("../ArchiveIssueButton", () => ({
  ArchiveIssueButton: () => <button data-testid="archive-button-stub">Archive</button>,
}));

vi.mock("../useArchiveIssue", () => ({
  useArchiveIssue: () => ({ archive: () => {}, isPending: false }),
}));

vi.mock("../EditableTitle", () => ({
  EditableTitle: ({
    issue,
    issueId,
    className,
  }: {
    issue: { title: string };
    issueId: string;
    className?: string;
  }) => <h1 className={className}>{issue.title || issueId}</h1>,
}));

vi.mock("../EditableDescription", () => ({
  EditableDescription: ({ issue }: { issue: { description: string } }) => (
    <div data-testid="editable-description-stub">{issue.description}</div>
  ),
}));

vi.mock("../IssueAssigneePicker", () => ({
  IssueAssigneePicker: ({
    issue,
    hideLabel,
  }: {
    issue: { assignee: Principal | null };
    hideLabel?: boolean;
  }) => {
    const assignee = issue.assignee;
    let label: string | null = null;
    let kind: "human" | "agent" | null = null;
    if (assignee) {
      if ("User" in assignee) {
        label = assignee.User.name;
        kind = "human";
      } else if ("Agent" in assignee) {
        label = assignee.Agent.name;
        kind = "agent";
      }
    }
    return (
      <div data-testid="issue-assignee-picker-stub" data-hide-label={hideLabel ? "true" : "false"}>
        {label ? (
          <span data-testid="avatar" data-kind={kind ?? undefined}>
            {label}
          </span>
        ) : (
          <span>Unassigned</span>
        )}
      </div>
    );
  },
}));

vi.mock("../../sessions/SessionList", () => ({
  SessionList: () => <div data-testid="session-list-stub" />,
}));

vi.mock("../../../components/MobileTabBar", () => ({
  MobileTabBar: () => <div data-testid="mobile-tab-bar-stub" />,
}));

vi.mock("react-router-dom", () => ({
  Link: ({ children, to }: { children: React.ReactNode; to: string }) => (
    <a href={to}>{children}</a>
  ),
  useNavigate: () => () => {},
}));

vi.mock("../IssueDetail.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

export function makeWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return ({ children }: { children: React.ReactNode }) =>
    React.createElement(QueryClientProvider, { client: queryClient }, children);
}
