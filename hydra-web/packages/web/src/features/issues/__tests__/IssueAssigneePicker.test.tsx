// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import type { ReactNode } from "react";
import type { Issue, Principal } from "@hydra/api";

interface MutationCapture {
  fn?: (input: Principal | null) => Promise<unknown>;
  lastInput?: Principal | null;
}
const mutationCapture: MutationCapture = {};

vi.mock("@tanstack/react-query", () => ({
  useMutation: (opts: { mutationFn: (input: Principal | null) => Promise<unknown> }) => {
    mutationCapture.fn = opts.mutationFn;
    return {
      mutate: (input: Principal | null) => {
        mutationCapture.lastInput = input;
      },
      isPending: false,
    };
  },
  useQueryClient: () => ({
    cancelQueries: vi.fn(),
    getQueryData: vi.fn(),
    setQueryData: vi.fn(),
    invalidateQueries: vi.fn(),
  }),
}));

const updateIssueMock = vi.fn<(issueId: string, body: unknown) => Promise<unknown>>(
  () => Promise.resolve({}),
);
vi.mock("../../../api/client", () => ({
  apiClient: {
    updateIssue: (issueId: string, body: unknown) => updateIssueMock(issueId, body),
  },
}));

vi.mock("../../../hooks/useAgents", () => ({
  useAgents: () => ({ data: [{ name: "swe" }, { name: "reviewer" }] }),
}));

vi.mock("../../../hooks/useUsers", () => ({
  useUsers: () => ({ data: [{ username: "alice" }, { username: "bob" }] }),
}));

vi.mock("../../toast/useToast", () => ({
  useToast: () => ({ addToast: vi.fn() }),
}));

vi.mock("@hydra/ui", () => ({
  Avatar: ({ name, kind }: { name: string; kind?: string }) => (
    <span data-testid="avatar" data-kind={kind}>{name}</span>
  ),
  Picker: ({
    label,
    hideLabel,
    value,
    open,
    onToggle,
    children,
    "data-testid": testId,
  }: {
    label: string;
    hideLabel?: boolean;
    value: ReactNode;
    open: boolean;
    onToggle: () => void;
    children: ReactNode;
    "data-testid"?: string;
  }) => (
    <div data-testid={testId ?? "picker"} data-hide-label={hideLabel ? "true" : "false"}>
      {!hideLabel && <span data-testid="picker-label">{label}</span>}
      <button type="button" onClick={onToggle} aria-expanded={open} aria-label={label}>
        {value}
      </button>
      {open && <div data-testid="picker-pop">{children}</div>}
    </div>
  ),
  PickerRow: ({
    active,
    onClick,
    children,
  }: {
    active?: boolean;
    onClick: () => void;
    children: ReactNode;
  }) => (
    <button
      type="button"
      onClick={onClick}
      data-active={active ? "true" : "false"}
      data-testid="picker-row"
    >
      {children}
    </button>
  ),
}));

vi.mock("../IssueAssigneePicker.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const { IssueAssigneePicker } = await import("../IssueAssigneePicker");

function makeIssue(assignee: Principal | null): Issue {
  return {
    type: "task",
    title: "Sample",
    description: "",
    creator: "alice",
    progress: "",
    status: { key: "open", label: "Open" },
    project_id: "j-defaul",
    assignee,
    dependencies: [],
    patches: [],
  } as unknown as Issue;
}

describe("IssueAssigneePicker", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mutationCapture.fn = undefined;
    mutationCapture.lastInput = undefined;
  });

  it("hides the caption when hideLabel is true", () => {
    render(<IssueAssigneePicker issueId="i-1" issue={makeIssue(null)} hideLabel />);
    expect(screen.queryByTestId("picker-label")).toBeNull();
    expect(screen.getByTestId("issue-assignee-picker").getAttribute("data-hide-label")).toBe("true");
  });

  it("renders the caption by default", () => {
    render(<IssueAssigneePicker issueId="i-1" issue={makeIssue(null)} />);
    expect(screen.getByTestId("picker-label").textContent).toBe("Assignee");
  });

  it("shows 'Unassigned' in the trigger when no assignee", () => {
    render(<IssueAssigneePicker issueId="i-1" issue={makeIssue(null)} hideLabel />);
    expect(screen.getByText("Unassigned")).toBeDefined();
  });

  it("renders the current agent assignee in the trigger pill", () => {
    render(
      <IssueAssigneePicker
        issueId="i-1"
        issue={makeIssue({ Agent: { name: "swe" } })}
        hideLabel
      />,
    );
    const avatar = screen.getByTestId("avatar");
    expect(avatar.getAttribute("data-kind")).toBe("agent");
    expect(avatar.textContent).toBe("swe");
  });

  it("opens the popover and lists agents + users when clicked", () => {
    render(<IssueAssigneePicker issueId="i-1" issue={makeIssue(null)} hideLabel />);
    fireEvent.click(screen.getByRole("button", { name: "Assignee" }));
    const pop = screen.getByTestId("picker-pop");
    expect(pop.textContent).toContain("swe");
    expect(pop.textContent).toContain("reviewer");
    expect(pop.textContent).toContain("alice");
    expect(pop.textContent).toContain("bob");
  });

  it("fires the mutation with the chosen agent principal", () => {
    render(<IssueAssigneePicker issueId="i-1" issue={makeIssue(null)} hideLabel />);
    fireEvent.click(screen.getByRole("button", { name: "Assignee" }));
    const rows = screen.getAllByTestId("picker-row");
    const sweRow = rows.find((row) => row.textContent?.includes("swe"));
    expect(sweRow).toBeDefined();
    fireEvent.click(sweRow!);
    expect(mutationCapture.lastInput).toEqual({ Agent: { name: "swe" } });
  });

  it("fires the mutation with null when the user picks 'Unassigned'", () => {
    render(
      <IssueAssigneePicker
        issueId="i-1"
        issue={makeIssue({ User: { name: "alice" } })}
        hideLabel
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Assignee" }));
    const rows = screen.getAllByTestId("picker-row");
    const unassignedRow = rows.find((row) =>
      row.textContent?.includes("Unassigned"),
    );
    expect(unassignedRow).toBeDefined();
    fireEvent.click(unassignedRow!);
    expect(mutationCapture.lastInput).toBeNull();
  });

  it("does not fire the mutation when the user re-picks the current assignee", () => {
    render(
      <IssueAssigneePicker
        issueId="i-1"
        issue={makeIssue({ Agent: { name: "swe" } })}
        hideLabel
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Assignee" }));
    const rows = screen.getAllByTestId("picker-row");
    const sweRow = rows.find(
      (row) =>
        row.textContent?.includes("swe") && row.getAttribute("data-active") === "true",
    );
    expect(sweRow).toBeDefined();
    fireEvent.click(sweRow!);
    expect(mutationCapture.lastInput).toBeUndefined();
  });

  it("sends the wire body with status mapped to its key and the new assignee", async () => {
    render(<IssueAssigneePicker issueId="i-1" issue={makeIssue(null)} hideLabel />);
    expect(mutationCapture.fn).toBeDefined();
    await mutationCapture.fn!({ User: { name: "bob" } });
    expect(updateIssueMock).toHaveBeenCalledTimes(1);
    const call = updateIssueMock.mock.calls[0];
    const body = call[1] as { issue: Record<string, unknown>; session_id: null };
    expect(body.session_id).toBeNull();
    expect(body.issue.status).toBe("open");
    expect(body.issue.assignee).toEqual({ User: { name: "bob" } });
  });
});
