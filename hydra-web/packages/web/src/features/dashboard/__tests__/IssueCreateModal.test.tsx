// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import type { ReactNode } from "react";

// --- Mocks ---

vi.mock("@tanstack/react-query", () => ({
  useMutation: () => ({ mutate: vi.fn(), isPending: false }),
  useQueryClient: () => ({ invalidateQueries: vi.fn() }),
}));

vi.mock("@hydra/ui", () => ({
  Avatar: ({ name }: { name: string }) => (
    <span data-testid={`avatar-${name}`}>{name}</span>
  ),
  Button: ({
    children,
    onClick,
    disabled,
  }: {
    children: ReactNode;
    onClick?: () => void;
    disabled?: boolean;
    variant?: string;
    size?: string;
  }) => (
    <button onClick={onClick} disabled={disabled}>
      {children}
    </button>
  ),
  Kbd: ({ children }: { children: ReactNode }) => <kbd>{children}</kbd>,
  TypeChip: ({ type }: { type: string }) => (
    <span data-testid={`type-chip-${type}`}>{type}</span>
  ),
  Icons: new Proxy(
    {},
    {
      get: (_t, prop) => () => <span data-testid={`icon-${String(prop)}`} />,
    },
  ),
}));

vi.mock("../../auth/useAuth", () => ({
  useAuth: () => ({ user: { actor: { type: "user", username: "testuser" } } }),
}));

vi.mock("../../../hooks/useRepositories", () => ({
  useRepositories: () => ({ data: [] }),
}));

vi.mock("../../../api/client", () => ({
  apiClient: { createIssue: vi.fn() },
}));

vi.mock("../../toast/useToast", () => ({
  useToast: () => ({ addToast: vi.fn() }),
}));

vi.mock("../../labels/LabelPicker", () => ({
  LabelPicker: () => <div data-testid="label-picker" />,
}));

vi.mock("../IssueCreateModal.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

// --- Import after mocks ---
const { IssueCreateModal } = await import("../IssueCreateModal");

const TITLE_PLACEHOLDER = "Issue title…";
const DESC_PLACEHOLDER = /describe the issue/i;

function openAssigneePicker() {
  // The Assignee pill is the only one with this label.
  const label = screen.getByText("Assignee");
  // Pill is the sibling button beneath the label inside the picker wrapper.
  const wrapper = label.parentElement!;
  const button = wrapper.querySelector("button")!;
  fireEvent.click(button);
}

describe("IssueCreateModal", () => {
  beforeEach(() => {
    sessionStorage.clear();
  });

  afterEach(() => {
    cleanup();
  });

  it("includes provided agent names in the Assignee picker", () => {
    render(
      <IssueCreateModal open onClose={() => {}} assignees={["pm", "reviewer", "swe"]} />,
    );

    openAssigneePicker();

    // "Unassigned" appears twice: once in the pill, once in the popover.
    // Avatars are mocked with a data-testid keyed off the name.
    expect(screen.getAllByText("Unassigned").length).toBeGreaterThan(0);
    expect(screen.getByTestId("avatar-pm")).toBeDefined();
    expect(screen.getByTestId("avatar-reviewer")).toBeDefined();
    expect(screen.getByTestId("avatar-swe")).toBeDefined();
  });

  it("renders only Unassigned when assignees is empty", () => {
    render(<IssueCreateModal open onClose={() => {}} assignees={[]} />);

    openAssigneePicker();

    // No avatars rendered for an empty assignee list — just Unassigned rows
    // (one in the pill, one in the popover).
    expect(screen.getAllByText("Unassigned").length).toBeGreaterThan(0);
    expect(screen.queryAllByTestId(/^avatar-/)).toHaveLength(0);
  });

  it("preserves drafts when the modal is dismissed via the close button", () => {
    const onClose = vi.fn();
    const { unmount } = render(
      <IssueCreateModal open onClose={onClose} assignees={[]} />,
    );

    fireEvent.change(screen.getByPlaceholderText(TITLE_PLACEHOLDER), {
      target: { value: "Draft title" },
    });
    fireEvent.change(screen.getByPlaceholderText(DESC_PLACEHOLDER), {
      target: { value: "Draft description" },
    });

    // ✕ close in the header — preserves drafts.
    fireEvent.click(screen.getByLabelText("Close"));
    expect(onClose).toHaveBeenCalledTimes(1);

    expect(sessionStorage.getItem("hydra:draft:issue-create-modal:title")).toBe(
      JSON.stringify("Draft title"),
    );
    expect(
      sessionStorage.getItem("hydra:draft:issue-create-modal:description"),
    ).toBe(JSON.stringify("Draft description"));

    unmount();
    render(<IssueCreateModal open onClose={onClose} assignees={[]} />);
    expect(
      (screen.getByPlaceholderText(TITLE_PLACEHOLDER) as HTMLInputElement).value,
    ).toBe("Draft title");
    expect(
      (screen.getByPlaceholderText(DESC_PLACEHOLDER) as HTMLTextAreaElement).value,
    ).toBe("Draft description");
  });

  it("clears drafts when the user clicks Cancel", () => {
    const onClose = vi.fn();
    const { unmount } = render(
      <IssueCreateModal open onClose={onClose} assignees={[]} />,
    );

    fireEvent.change(screen.getByPlaceholderText(TITLE_PLACEHOLDER), {
      target: { value: "Draft title" },
    });
    fireEvent.change(screen.getByPlaceholderText(DESC_PLACEHOLDER), {
      target: { value: "Draft description" },
    });

    fireEvent.click(screen.getByText("Cancel"));
    expect(onClose).toHaveBeenCalledTimes(1);

    expect(sessionStorage.getItem("hydra:draft:issue-create-modal:title")).toBe(null);
    expect(
      sessionStorage.getItem("hydra:draft:issue-create-modal:description"),
    ).toBe(null);

    unmount();
    render(<IssueCreateModal open onClose={onClose} assignees={[]} />);
    expect(
      (screen.getByPlaceholderText(TITLE_PLACEHOLDER) as HTMLInputElement).value,
    ).toBe("");
    expect(
      (screen.getByPlaceholderText(DESC_PLACEHOLDER) as HTMLTextAreaElement).value,
    ).toBe("");
  });
});
