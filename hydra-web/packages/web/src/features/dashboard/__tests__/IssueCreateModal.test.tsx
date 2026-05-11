import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
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
    children,
  }: {
    open: boolean;
    title?: string;
    children: ReactNode;
  }) =>
    open ? (
      <div role="dialog" aria-label={title}>
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
});
