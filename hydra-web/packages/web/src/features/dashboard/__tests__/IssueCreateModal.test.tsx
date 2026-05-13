import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import type { ReactNode } from "react";

// --- Mocks ---

vi.mock("@tanstack/react-query", () => ({
  useMutation: () => ({ mutate: vi.fn(), isPending: false }),
  useQueryClient: () => ({ invalidateQueries: vi.fn() }),
}));

interface SelectOption {
  value: string;
  label: string;
}

vi.mock("@hydra/ui", () => ({
  Modal: ({
    open,
    title,
    onClose,
    children,
  }: {
    open: boolean;
    title?: string;
    onClose: () => void;
    children: ReactNode;
  }) =>
    open ? (
      <div role="dialog" aria-label={title}>
        <button aria-label="Close" onClick={onClose}>
          Close
        </button>
        {children}
      </div>
    ) : null,
  Button: ({ children, onClick }: { children: ReactNode; onClick?: () => void }) => (
    <button onClick={onClick}>{children}</button>
  ),
  Input: ({ label, value, onChange }: { label?: string; value: string; onChange: (e: { target: { value: string } }) => void }) => (
    <label>
      {label}
      <input value={value} onChange={onChange} />
    </label>
  ),
  Textarea: ({ label, value, onChange }: { label?: string; value: string; onChange: (e: { target: { value: string } }) => void }) => (
    <label>
      {label}
      <textarea value={value} onChange={onChange} />
    </label>
  ),
  Select: ({
    label,
    options,
    value,
    onChange,
  }: {
    label?: string;
    options: SelectOption[];
    value: string;
    onChange: (e: { target: { value: string } }) => void;
  }) => (
    <label>
      {label}
      <select value={value} onChange={onChange}>
        {options.map((opt) => (
          <option key={opt.value} value={opt.value}>
            {opt.label}
          </option>
        ))}
      </select>
    </label>
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

vi.mock("../../../components/LargeModal.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

// --- Import after mocks ---
const { IssueCreateModal } = await import("../IssueCreateModal");

describe("IssueCreateModal", () => {
  beforeEach(() => {
    sessionStorage.clear();
  });

  afterEach(() => {
    cleanup();
  });

  it("includes provided agent names in the Assignee dropdown after expanding More options", () => {
    render(
      <IssueCreateModal
        open
        onClose={() => {}}
        assignees={["pm", "reviewer", "swe"]}
      />,
    );

    // Expand "More options" to reveal the Assignee select
    fireEvent.click(screen.getByText("More options"));

    const select = screen.getByLabelText("Assignee") as HTMLSelectElement;
    const optionValues = Array.from(select.options).map((o) => o.value);
    const optionLabels = Array.from(select.options).map((o) => o.textContent);

    expect(optionValues).toEqual(["", "pm", "reviewer", "swe"]);
    expect(optionLabels).toEqual(["Unassigned", "pm", "reviewer", "swe"]);
  });

  it("renders only Unassigned when assignees is empty", () => {
    render(<IssueCreateModal open onClose={() => {}} assignees={[]} />);

    fireEvent.click(screen.getByText("More options"));

    const select = screen.getByLabelText("Assignee") as HTMLSelectElement;
    const optionLabels = Array.from(select.options).map((o) => o.textContent);
    expect(optionLabels).toEqual(["Unassigned"]);
  });

  it("preserves drafts when the modal is dismissed (Modal onClose)", () => {
    const onClose = vi.fn();
    const { unmount } = render(
      <IssueCreateModal open onClose={onClose} assignees={[]} />,
    );

    fireEvent.change(screen.getByLabelText("Title"), {
      target: { value: "Draft title" },
    });
    fireEvent.change(screen.getByLabelText("Description"), {
      target: { value: "Draft description" },
    });

    // Dismiss via the Modal's Close affordance (Escape / backdrop / header ✕).
    fireEvent.click(screen.getByLabelText("Close"));
    expect(onClose).toHaveBeenCalledTimes(1);

    // sessionStorage retains the drafts.
    expect(sessionStorage.getItem("hydra:draft:issue-create-modal:title")).toBe(
      JSON.stringify("Draft title"),
    );
    expect(
      sessionStorage.getItem("hydra:draft:issue-create-modal:description"),
    ).toBe(JSON.stringify("Draft description"));

    // Remount and verify the drafts are restored into the inputs.
    unmount();
    render(<IssueCreateModal open onClose={onClose} assignees={[]} />);
    expect((screen.getByLabelText("Title") as HTMLInputElement).value).toBe(
      "Draft title",
    );
    expect(
      (screen.getByLabelText("Description") as HTMLTextAreaElement).value,
    ).toBe("Draft description");
  });

  it("clears drafts when the user clicks Cancel", () => {
    const onClose = vi.fn();
    const { unmount } = render(
      <IssueCreateModal open onClose={onClose} assignees={[]} />,
    );

    fireEvent.change(screen.getByLabelText("Title"), {
      target: { value: "Draft title" },
    });
    fireEvent.change(screen.getByLabelText("Description"), {
      target: { value: "Draft description" },
    });

    fireEvent.click(screen.getByText("Cancel"));
    expect(onClose).toHaveBeenCalledTimes(1);

    expect(sessionStorage.getItem("hydra:draft:issue-create-modal:title")).toBe(
      null,
    );
    expect(
      sessionStorage.getItem("hydra:draft:issue-create-modal:description"),
    ).toBe(null);

    unmount();
    render(<IssueCreateModal open onClose={onClose} assignees={[]} />);
    expect((screen.getByLabelText("Title") as HTMLInputElement).value).toBe("");
    expect(
      (screen.getByLabelText("Description") as HTMLTextAreaElement).value,
    ).toBe("");
  });
});
