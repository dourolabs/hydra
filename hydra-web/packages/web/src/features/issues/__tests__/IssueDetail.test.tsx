import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type { Form, FormResponse, IssueVersionRecord } from "@hydra/api";

// --- Hook mocks ---
vi.mock("../useIssue", () => ({
  useIssue: () => ({ data: undefined }),
}));

vi.mock("../../sessions/useSessionsByIssue", () => ({
  useSessionsByIssue: () => ({ data: [] }),
}));

vi.mock("../../dashboard/useSessionDuration", () => ({
  useSessionDuration: () => ({ durationText: "", isRunning: false }),
}));

// --- Sibling component stubs ---
vi.mock("../IssueRightPanel", () => ({
  IssueRightPanel: () => <div data-testid="right-panel-stub" />,
}));

vi.mock("../IssueUpdateModal", () => ({
  IssueUpdateModal: () => null,
}));

vi.mock("../FeedbackModal", () => ({
  FeedbackModal: () => null,
}));

vi.mock("../../sessions/SessionList", () => ({
  SessionList: () => <div data-testid="session-list-stub" />,
}));

vi.mock("../../../components/MobileTabBar", () => ({
  MobileTabBar: () => <div data-testid="mobile-tab-bar-stub" />,
}));

// --- @hydra/ui stubs (minimal renderings sufficient for the assertions) ---
vi.mock("@hydra/ui", () => ({
  Badge: ({ status }: { status: string }) => <span data-testid={`badge-${status}`}>{status}</span>,
  TypeChip: ({ type }: { type: string }) => <span data-testid={`type-${type}`}>{type}</span>,
  Button: ({
    children,
    onClick,
    disabled,
  }: {
    children: React.ReactNode;
    onClick?: () => void;
    disabled?: boolean;
  }) => (
    <button onClick={onClick} disabled={disabled}>
      {children}
    </button>
  ),
  Input: (props: React.InputHTMLAttributes<HTMLInputElement>) => <input {...props} />,
  Select: ({
    options,
    ...rest
  }: React.SelectHTMLAttributes<HTMLSelectElement> & {
    options: { value: string; label: string }[];
  }) => (
    <select {...rest}>
      {options.map((o) => (
        <option key={o.value} value={o.value}>
          {o.label}
        </option>
      ))}
    </select>
  ),
  Textarea: (props: React.TextareaHTMLAttributes<HTMLTextAreaElement>) => <textarea {...props} />,
  MarkdownViewer: ({ content }: { content: string }) => <div>{content}</div>,
}));

vi.mock("react-router-dom", () => ({
  Link: ({ children, to }: { children: React.ReactNode; to: string }) => (
    <a href={to}>{children}</a>
  ),
}));

vi.mock("../IssueDetail.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("../FormPanel.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const { IssueDetail } = await import("../IssueDetail");

function makeForm(): Form {
  return {
    prompt: "Please review the OAuth2 migration proposal",
    fields: [
      {
        key: "decision",
        label: "Decision",
        input: {
          type: "select",
          radio: true,
          options: [
            { value: "approve", label: "Approve" },
            { value: "reject", label: "Reject" },
          ],
        },
      },
    ],
    actions: [
      {
        id: "submit",
        label: "Submit review",
        style: "primary",
        requires: ["decision"],
        effect: { type: "record_only" },
      },
    ],
  };
}

function makeResponse(): FormResponse {
  return {
    action_id: "submit",
    actor: { Username: "alice" },
    values: { decision: "approve" },
    submitted_at: "2026-05-14T18:42:00.000Z",
  };
}

function makeRecord(overrides: {
  form?: Form | null;
  form_response?: FormResponse | null;
}): IssueVersionRecord {
  return {
    issue_id: "i-1",
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    labels: [],
    issue: {
      type: "review-request",
      title: "Sample",
      description: "",
      creator: "alice",
      status: "open",
      progress: "",
      dependencies: [],
      patches: [],
      labels: [],
      form: overrides.form ?? null,
      form_response: overrides.form_response ?? null,
    },
  } as unknown as IssueVersionRecord;
}

function makeWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return ({ children }: { children: React.ReactNode }) =>
    React.createElement(QueryClientProvider, { client: queryClient }, children);
}

describe("IssueDetail FormPanel rendering", () => {
  beforeEach(() => vi.clearAllMocks());

  it("renders FormPanel with prompt and action button when issue.form is set", () => {
    render(<IssueDetail record={makeRecord({ form: makeForm() })} />, {
      wrapper: makeWrapper(),
    });

    expect(screen.getByText("Please review the OAuth2 migration proposal")).toBeDefined();
    expect(screen.getByRole("button", { name: "Submit review" })).toBeDefined();
  });

  it("does not render FormPanel when issue.form is null", () => {
    render(<IssueDetail record={makeRecord({ form: null })} />, {
      wrapper: makeWrapper(),
    });

    expect(screen.queryByText("Please review the OAuth2 migration proposal")).toBeNull();
    expect(screen.queryByRole("button", { name: "Submit review" })).toBeNull();
  });

  it("renders FormPanel in read-only mode (no action buttons) when issue.form_response is set", () => {
    render(
      <IssueDetail
        record={makeRecord({
          form: makeForm(),
          form_response: makeResponse(),
        })}
      />,
      { wrapper: makeWrapper() },
    );

    // Prompt still rendered.
    expect(screen.getByText("Please review the OAuth2 migration proposal")).toBeDefined();
    // But action buttons are gone in the read-only branch.
    expect(screen.queryByRole("button", { name: "Submit review" })).toBeNull();
  });
});
