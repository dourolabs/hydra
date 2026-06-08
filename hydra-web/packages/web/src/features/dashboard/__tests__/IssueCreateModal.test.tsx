// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import type { ReactNode } from "react";

// --- Mocks ---

// Capture the mutationFn passed by useFormModal so tests can drive
// handleSubmit → mutationFn directly and assert on the createIssue body.
type CapturedMutation = { mutationFn?: (input: unknown) => Promise<unknown> };
const capturedMutation: CapturedMutation = {};

vi.mock("@tanstack/react-query", () => ({
  useMutation: (opts: { mutationFn?: (input: unknown) => Promise<unknown> }) => {
    capturedMutation.mutationFn = opts.mutationFn;
    return {
      mutate: (input: unknown) => {
        opts.mutationFn?.(input);
      },
      isPending: false,
    };
  },
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
  Picker: ({
    label,
    value,
    open,
    onToggle,
    children,
  }: {
    label: string;
    value: ReactNode;
    open: boolean;
    onToggle: () => void;
    children: ReactNode;
  }) => (
    <div data-testid={`picker-${label.toLowerCase()}`}>
      <span>{label}</span>
      <button type="button" onClick={onToggle} aria-expanded={open} aria-label={label}>
        {value}
      </button>
      {open && <div data-testid={`picker-pop-${label.toLowerCase()}`}>{children}</div>}
    </div>
  ),
  PickerRow: ({
    onClick,
    children,
  }: {
    active?: boolean;
    onClick: () => void;
    children: ReactNode;
  }) => (
    <button type="button" onClick={onClick}>
      {children}
    </button>
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

// `createIssue` is invoked through the mutation in handleSubmit. We capture
// the body it receives so tests can assert `project_id` / `status` plumbing.
const createIssueMock = vi.fn<(body: unknown) => Promise<{ issue_id: string }>>(
  () => Promise.resolve({ issue_id: "i-new" }),
);
vi.mock("../../../api/client", () => ({
  apiClient: { createIssue: (body: unknown) => createIssueMock(body) },
}));

const useProjectsMock = vi.fn();
const useProjectStatusesMock = vi.fn();
vi.mock("../../projects/useProjects", () => ({
  useProjects: () => useProjectsMock(),
  useProjectStatuses: (projectId: string | null) =>
    useProjectStatusesMock(projectId),
}));

vi.mock("../../projects/StatusChip", () => ({
  StatusChip: ({
    definition,
    fallbackKey,
  }: {
    definition?: { key: string; label: string } | null;
    fallbackKey?: string | null;
  }) => (
    <span data-testid={`status-chip-${definition?.key ?? fallbackKey ?? "empty"}`}>
      {definition?.label ?? fallbackKey ?? ""}
    </span>
  ),
}));

vi.mock("../../toast/useToast", () => ({
  useToast: () => ({ addToast: vi.fn() }),
}));

const SEEDED_LABELS = [
  { label_id: "l1", name: "bug", color: "#e74c3c", hidden: false },
  { label_id: "l2", name: "feature", color: "#3498db", hidden: false },
  { label_id: "l3", name: "infra", color: "#2ecc71", hidden: false },
];
const SEEDED_PALETTE = [
  "#e74c3c",
  "#e67e22",
  "#f1c40f",
  "#2ecc71",
  "#1abc9c",
  "#3498db",
  "#9b59b6",
  "#e91e63",
  "#795548",
  "#607d8b",
];

vi.mock("../../labels/useLabels", () => ({
  useLabels: () => ({ data: SEEDED_LABELS }),
}));

vi.mock("../../labels/LabelPicker", () => ({
  LABEL_COLOR_PALETTE: SEEDED_PALETTE,
}));

vi.mock("../IssueCreateModal.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

// --- Import after mocks ---
const { IssueCreateModal } = await import("../IssueCreateModal");

const TITLE_PLACEHOLDER = "Issue title…";
const DESC_PLACEHOLDER = /describe the issue/i;

function openPickerByLabel(labelText: string) {
  // The picker label uniquely identifies the picker; the pill is its sibling.
  const label = screen.getByText(labelText);
  const wrapper = label.parentElement!;
  const button = wrapper.querySelector("button")!;
  fireEvent.click(button);
}

function openAssigneePicker() {
  openPickerByLabel("Assignee");
}

const SEEDED_PROJECTS = [
  {
    project_id: "j-engv2",
    version: 1,
    project: {
      key: "engineering-v2",
      name: "Engineering v2",
      statuses: [],
      creator: "alice",
    },
  },
];
const ENG_V2_STATUSES = {
  statuses: [
    { key: "inbox", label: "Inbox", color: "#aaa" },
    { key: "backlog", label: "Backlog", color: "#bbb" },
    { key: "pending", label: "Pending", color: "#ccc" },
  ],
};
const DEFAULT_STATUSES = {
  statuses: [
    { key: "open", label: "Open", color: "#111" },
    { key: "in-progress", label: "In progress", color: "#222" },
  ],
};

describe("IssueCreateModal", () => {
  beforeEach(() => {
    sessionStorage.clear();
    createIssueMock.mockClear();
    capturedMutation.mutationFn = undefined;
    useProjectsMock.mockReturnValue({ data: SEEDED_PROJECTS });
    useProjectStatusesMock.mockImplementation((projectId: string | null) =>
      projectId === "j-engv2"
        ? { data: ENG_V2_STATUSES }
        : { data: DEFAULT_STATUSES },
    );
  });

  afterEach(() => {
    cleanup();
  });

  it("includes provided agent names in the Assignee picker", () => {
    render(
      <IssueCreateModal
        open
        onClose={() => {}}
        assignees={{ agents: ["pm", "reviewer", "swe"], users: [] }}
      />,
    );

    openAssigneePicker();

    // "Unassigned" appears twice: once in the pill, once in the popover.
    // Avatars are mocked with a data-testid keyed off the name.
    expect(screen.getAllByText("Unassigned").length).toBeGreaterThan(0);
    expect(screen.getByTestId("avatar-pm")).toBeDefined();
    expect(screen.getByTestId("avatar-reviewer")).toBeDefined();
    expect(screen.getByTestId("avatar-swe")).toBeDefined();
  });

  it("renders both Agents and Users sections when both lists are non-empty", () => {
    render(
      <IssueCreateModal
        open
        onClose={() => {}}
        assignees={{ agents: ["swe"], users: ["alice"] }}
      />,
    );

    openAssigneePicker();

    expect(screen.getByText("Agents")).toBeDefined();
    expect(screen.getByText("Users")).toBeDefined();
    expect(screen.getByTestId("avatar-swe")).toBeDefined();
    expect(screen.getByTestId("avatar-alice")).toBeDefined();
  });

  it("renders only Unassigned when assignees is empty", () => {
    render(<IssueCreateModal open onClose={() => {}} assignees={{ agents: [], users: [] }} />);

    openAssigneePicker();

    // No avatars rendered for an empty assignee list — just Unassigned rows
    // (one in the pill, one in the popover).
    expect(screen.getAllByText("Unassigned").length).toBeGreaterThan(0);
    expect(screen.queryAllByTestId(/^avatar-/)).toHaveLength(0);
  });

  it("preserves drafts when the modal is dismissed via the close button", () => {
    const onClose = vi.fn();
    const { unmount } = render(
      <IssueCreateModal open onClose={onClose} assignees={{ agents: [], users: [] }} />,
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
    render(<IssueCreateModal open onClose={onClose} assignees={{ agents: [], users: [] }} />);
    expect(
      (screen.getByPlaceholderText(TITLE_PLACEHOLDER) as HTMLInputElement).value,
    ).toBe("Draft title");
    expect(
      (screen.getByPlaceholderText(DESC_PLACEHOLDER) as HTMLTextAreaElement).value,
    ).toBe("Draft description");
  });

  it("renders all label rows in the Labels picker popover when opened", () => {
    render(<IssueCreateModal open onClose={() => {}} assignees={{ agents: [], users: [] }} />);

    openPickerByLabel("Labels");

    // Search input + every seeded label row should be reachable from `screen`
    // even though the popover is rendered into document.body via a portal.
    expect(screen.getByPlaceholderText("Search or create…")).toBeDefined();
    for (const label of SEEDED_LABELS) {
      expect(screen.getByText(label.name)).toBeDefined();
    }
  });

  it("renders all color swatches in the Labels picker create-new section", () => {
    render(<IssueCreateModal open onClose={() => {}} assignees={{ agents: [], users: [] }} />);

    openPickerByLabel("Labels");

    // Typing in the search box reveals the create-new section + palette.
    fireEvent.change(screen.getByPlaceholderText("Search or create…"), {
      target: { value: "needs-triage" },
    });

    expect(screen.getByText(/Create [“"]needs-triage[”"]/)).toBeDefined();
    // All ten color swatches must render and be queryable, including the last one.
    const swatches = screen.getAllByRole("radio");
    expect(swatches).toHaveLength(SEEDED_PALETTE.length);

    // Clicking the last swatch updates the active radio — proves it's clickable
    // (i.e. not clipped/inert in the DOM).
    const lastSwatch = swatches[swatches.length - 1];
    fireEvent.click(lastSwatch);
    expect(lastSwatch.getAttribute("aria-checked")).toBe("true");
  });

  it("renders the Type picker rows when opened", () => {
    render(<IssueCreateModal open onClose={() => {}} assignees={{ agents: [], users: [] }} />);

    openPickerByLabel("Type");

    // Each issue type renders as its own TypeChip row in the popover (in
    // addition to the trigger pill chip).
    expect(screen.getAllByTestId("type-chip-task").length).toBeGreaterThan(1);
    expect(screen.getByTestId("type-chip-bug")).toBeDefined();
    expect(screen.getByTestId("type-chip-feature")).toBeDefined();
    expect(screen.getByTestId("type-chip-chore")).toBeDefined();
  });

  it("clears drafts when the user clicks Cancel", () => {
    const onClose = vi.fn();
    const { unmount } = render(
      <IssueCreateModal open onClose={onClose} assignees={{ agents: [], users: [] }} />,
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
    render(<IssueCreateModal open onClose={onClose} assignees={{ agents: [], users: [] }} />);
    expect(
      (screen.getByPlaceholderText(TITLE_PLACEHOLDER) as HTMLInputElement).value,
    ).toBe("");
    expect(
      (screen.getByPlaceholderText(DESC_PLACEHOLDER) as HTMLTextAreaElement).value,
    ).toBe("");
  });

  it("renders Project + Status picker testids alongside the existing pickers", () => {
    render(
      <IssueCreateModal open onClose={() => {}} assignees={{ agents: [], users: [] }} />,
    );
    expect(screen.getByTestId("issue-create-project-picker")).toBeDefined();
    expect(screen.getByTestId("issue-create-status-picker")).toBeDefined();
  });

  it("submits with no project_id and status=\"open\" when nothing is selected", () => {
    render(
      <IssueCreateModal open onClose={() => {}} assignees={{ agents: [], users: [] }} />,
    );

    fireEvent.change(screen.getByPlaceholderText(DESC_PLACEHOLDER), {
      target: { value: "needs a fix" },
    });
    fireEvent.click(screen.getByText(/Create issue/));

    expect(createIssueMock).toHaveBeenCalledTimes(1);
    const body = createIssueMock.mock.calls[0][0] as {
      issue: { status: string; project_id?: string };
    };
    expect(body.issue.status).toBe("open");
    expect(body.issue.project_id).toBeUndefined();
  });

  it("submits project_id + chosen status when both pickers are set", () => {
    render(
      <IssueCreateModal open onClose={() => {}} assignees={{ agents: [], users: [] }} />,
    );

    fireEvent.change(screen.getByPlaceholderText(DESC_PLACEHOLDER), {
      target: { value: "build the thing" },
    });

    // Pick engineering-v2 from the Project picker.
    openPickerByLabel("Project");
    fireEvent.click(screen.getByText("engineering-v2"));

    // The Status picker should now expose engineering-v2's status list.
    openPickerByLabel("Status");
    // ENG_V2_STATUSES contains `backlog`; pick it.
    const backlogChip = screen.getAllByTestId("status-chip-backlog")[0];
    fireEvent.click(backlogChip);

    fireEvent.click(screen.getByText(/Create issue/));

    expect(createIssueMock).toHaveBeenCalledTimes(1);
    const body = createIssueMock.mock.calls[0][0] as {
      issue: { status: string; project_id?: string };
    };
    expect(body.issue.project_id).toBe("j-engv2");
    expect(body.issue.status).toBe("backlog");
  });

  it("submits the legacy default status when the Status picker isn't touched", () => {
    render(
      <IssueCreateModal open onClose={() => {}} assignees={{ agents: [], users: [] }} />,
    );

    fireEvent.change(screen.getByPlaceholderText(DESC_PLACEHOLDER), {
      target: { value: "fresh issue" },
    });

    openPickerByLabel("Project");
    fireEvent.click(screen.getByText("engineering-v2"));

    fireEvent.click(screen.getByText(/Create issue/));

    expect(createIssueMock).toHaveBeenCalledTimes(1);
    const body = createIssueMock.mock.calls[0][0] as {
      issue: { status: string; project_id?: string };
    };
    expect(body.issue.project_id).toBe("j-engv2");
    expect(body.issue.status).toBe("open");
  });
});
