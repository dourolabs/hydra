import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import React from "react";
import type { Form, FormResponse, IssueVersionRecord, Principal } from "@hydra/api";
import { makeWrapper } from "./issueDetailHarness";

// --- Test-file-only stubs (not shared with sibling IssueDetail tests) ---

vi.mock("../IssueProjectPicker", () => ({
  IssueProjectPicker: () => <div data-testid="issue-project-picker-stub" />,
}));

vi.mock("../IssueStatusPicker", () => ({
  IssueStatusPicker: () => <div data-testid="issue-status-picker-stub" />,
}));

vi.mock("@hydra/ui", () => ({
  Avatar: ({ name, kind }: { name: string; kind?: string }) => (
    <span data-testid="avatar" data-kind={kind}>
      {name}
    </span>
  ),
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
  Icons: new Proxy(
    {},
    {
      get: () => () => <span aria-hidden="true" />,
    },
  ),
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
    actor: { User: { name: "alice" } },
    values: { decision: "approve" },
    submitted_at: "2026-05-14T18:42:00.000Z",
  };
}

function makeRecord(overrides: {
  form?: Form | null;
  form_response?: FormResponse | null;
  assignee?: Principal | null;
  archived?: boolean;
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
      assignee: overrides.assignee ?? null,
      dependencies: [],
      patches: [],
      labels: [],
      form: overrides.form ?? null,
      form_response: overrides.form_response ?? null,
      archived: overrides.archived ?? false,
    },
  } as unknown as IssueVersionRecord;
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

describe("IssueDetail assignee rendering", () => {
  beforeEach(() => vi.clearAllMocks());

  it("renders an avatar and name for a human assignee", () => {
    render(<IssueDetail record={makeRecord({ assignee: { User: { name: "alice" } } })} />, {
      wrapper: makeWrapper(),
    });

    const avatar = screen.getByTestId("avatar");
    expect(avatar).toBeDefined();
    expect(avatar.getAttribute("data-kind")).toBe("human");
    expect(avatar.textContent).toBe("alice");
    expect(screen.queryByText("Unassigned")).toBeNull();
    expect(screen.queryByText(/opened by/i)).toBeNull();
  });

  it("renders an avatar with agent kind for an Agent assignee", () => {
    render(<IssueDetail record={makeRecord({ assignee: { Agent: { name: "swe" } } })} />, {
      wrapper: makeWrapper(),
    });

    const avatar = screen.getByTestId("avatar");
    expect(avatar.getAttribute("data-kind")).toBe("agent");
    expect(avatar.textContent).toBe("swe");
  });

  it("renders 'Unassigned' when the issue has no assignee", () => {
    render(<IssueDetail record={makeRecord({ assignee: null })} />, {
      wrapper: makeWrapper(),
    });

    expect(screen.getByText("Unassigned")).toBeDefined();
    expect(screen.queryByTestId("avatar")).toBeNull();
  });
});

describe("IssueDetail archived badge", () => {
  beforeEach(() => vi.clearAllMocks());

  it("renders an Archived badge when issue.archived is true", () => {
    render(<IssueDetail record={makeRecord({ archived: true })} />, {
      wrapper: makeWrapper(),
    });

    expect(screen.getByTestId("badge-archived")).toBeDefined();
  });

  it("does not render an Archived badge for non-archived issues", () => {
    render(<IssueDetail record={makeRecord({ archived: false })} />, {
      wrapper: makeWrapper(),
    });

    expect(screen.queryByTestId("badge-archived")).toBeNull();
  });
});

describe("IssueDetail CommentsPanel placement", () => {
  beforeEach(() => vi.clearAllMocks());

  it("always renders CommentsPanel scoped to the issue id", () => {
    render(<IssueDetail record={makeRecord({})} />, { wrapper: makeWrapper() });
    const panel = screen.getByTestId("comments-panel-stub");
    expect(panel).toBeDefined();
    expect(panel.getAttribute("data-issue-id")).toBe("i-1");
  });
});

describe("IssueDetail title and overflow menu", () => {
  beforeEach(() => vi.clearAllMocks());

  it("renders the issue title as an h1 heading", () => {
    render(<IssueDetail record={makeRecord({})} />, { wrapper: makeWrapper() });
    // The H1 stays in the detail body for desktop; CSS hides it on mobile
    // (where the breadcrumb's trailing crumb shows the title instead).
    const heading = screen.getByRole("heading", { name: "Sample" });
    expect(heading.tagName).toBe("H1");
  });

  it("renders an overflow trigger and archive menu item", () => {
    render(<IssueDetail record={makeRecord({})} />, { wrapper: makeWrapper() });

    const trigger = screen.getByTestId("issue-overflow-trigger");
    expect(trigger).toBeDefined();

    fireEvent.click(trigger);

    expect(screen.getByTestId("issue-overflow-archive")).toBeDefined();
    // No conversation menu item without a live spawned conversation.
    expect(screen.queryByTestId("issue-overflow-conversation")).toBeNull();
  });

  it("omits the archive menu item when the issue is already archived", () => {
    render(<IssueDetail record={makeRecord({ archived: true })} />, {
      wrapper: makeWrapper(),
    });

    fireEvent.click(screen.getByTestId("issue-overflow-trigger"));

    expect(screen.queryByTestId("issue-overflow-archive")).toBeNull();
  });
});
