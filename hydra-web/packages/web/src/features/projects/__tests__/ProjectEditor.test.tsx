// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { ReactNode } from "react";

const mutateSpy = vi.fn();
const addToastSpy = vi.fn();

vi.mock("react-router-dom", () => ({
  useNavigate: () => vi.fn(),
}));

vi.mock("@tanstack/react-query", () => ({
  // Both the save and delete mutations resolve to the same stable spy;
  // tests in this file never trigger the delete flow, so collapsing the
  // two onto a single spy keeps the assertion target predictable across
  // re-renders.
  useMutation: () => ({ mutate: mutateSpy, isPending: false }),
  useQueryClient: () => ({
    cancelQueries: vi.fn(),
    getQueryData: vi.fn(),
    setQueryData: vi.fn(),
    invalidateQueries: vi.fn(),
  }),
}));

vi.mock("@hydra/ui", () => ({
  Button: ({
    children,
    onClick,
    disabled,
    "data-testid": testId,
  }: {
    children: ReactNode;
    onClick?: () => void;
    disabled?: boolean;
    variant?: string;
    size?: string;
    "data-testid"?: string;
  }) => (
    <button onClick={onClick} disabled={disabled} data-testid={testId}>
      {children}
    </button>
  ),
  Input: ({
    label,
    value,
    onChange,
    placeholder,
    "data-testid": testId,
  }: {
    label?: string;
    value: string;
    onChange: (e: { target: { value: string } }) => void;
    placeholder?: string;
    required?: boolean;
    disabled?: boolean;
    "data-testid"?: string;
  }) => (
    <label>
      {label}
      <input
        value={value}
        onChange={(e) => onChange({ target: { value: e.target.value } })}
        placeholder={placeholder}
        data-testid={testId}
      />
    </label>
  ),
  Select: ({
    label,
    options,
    value,
    onChange,
    "data-testid": testId,
  }: {
    label?: string;
    options: { value: string; label: string }[];
    value: string;
    onChange: (e: { target: { value: string } }) => void;
    "data-testid"?: string;
  }) => (
    <label>
      {label}
      <select
        value={value}
        onChange={(e) => onChange({ target: { value: e.target.value } })}
        data-testid={testId}
      >
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
    </label>
  ),
}));

vi.mock("../../../api/client", () => ({
  apiClient: {
    createProject: vi.fn(async (req: unknown) => req),
    updateProject: vi.fn(async (_id: string, req: unknown) => req),
    deleteProject: vi.fn(async () => ({ project_id: "j-x", version: 1 })),
  },
}));

vi.mock("../../toast/useToast", () => ({
  useToast: () => ({ addToast: addToastSpy }),
}));

vi.mock("../../../hooks/useAgents", () => ({
  useAgents: () => ({ data: [] }),
}));

vi.mock("../../../hooks/useUsers", () => ({
  useUsers: () => ({ data: [] }),
}));

vi.mock("../../../components/ColorPicker", () => ({
  ColorPicker: ({ value }: { value: string }) => (
    <span data-testid={`color-${value}`}>{value}</span>
  ),
  LABEL_COLOR_PALETTE: ["#111111", "#222222", "#333333", "#444444", "#555555", "#666666"],
}));

vi.mock(
  "../../../components/DeleteConfirmModal/DeleteConfirmModal",
  () => ({
    DeleteConfirmModal: () => null,
  }),
);

vi.mock("../ProjectEditor.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const { ProjectEditor } = await import("../ProjectEditor");

const PROMPT_TESTID = "project-editor-prompt-path";
const SAVE_TESTID = "project-editor-save";

describe("ProjectEditor — prompt_path", () => {
  beforeEach(() => {
    addToastSpy.mockReset();
    mutateSpy.mockReset();
  });

  afterEach(() => {
    cleanup();
  });

  it("renders the Prompt path input on the project form", () => {
    render(<ProjectEditor creator="alice" />);
    expect(screen.getByTestId(PROMPT_TESTID)).toBeDefined();
  });

  it("renders the Prompt path input on every status card", () => {
    render(<ProjectEditor creator="alice" />);
    // The default new-project flow seeds three statuses; each card should
    // expose a per-status prompt-path input.
    expect(screen.getByTestId("status-editor-prompt-path-0")).toBeDefined();
    expect(screen.getByTestId("status-editor-prompt-path-1")).toBeDefined();
    expect(screen.getByTestId("status-editor-prompt-path-2")).toBeDefined();
  });

  it("sends a populated prompt_path through to the mutation", () => {
    render(<ProjectEditor creator="alice" />);

    fireEvent.change(screen.getByTestId("project-editor-key"), {
      target: { value: "engineering" },
    });
    fireEvent.change(screen.getByTestId("project-editor-name"), {
      target: { value: "Engineering" },
    });
    fireEvent.change(screen.getByTestId(PROMPT_TESTID), {
      target: { value: "/projects/engineering/prompt.md" },
    });
    fireEvent.change(screen.getByTestId("status-editor-prompt-path-0"), {
      target: { value: "/projects/engineering/statuses/open.md" },
    });

    fireEvent.click(screen.getByTestId(SAVE_TESTID));

    expect(mutateSpy).toHaveBeenCalledTimes(1);
    const req = mutateSpy.mock.calls[0][0] as {
      project: {
        prompt_path: string | null;
        statuses: { key: string; prompt_path: string | null }[];
      };
    };
    expect(req.project.prompt_path).toBe("/projects/engineering/prompt.md");
    expect(req.project.statuses[0]?.prompt_path).toBe(
      "/projects/engineering/statuses/open.md",
    );
    // Other statuses default to null when left blank.
    expect(req.project.statuses[1]?.prompt_path).toBeNull();
    expect(req.project.statuses[2]?.prompt_path).toBeNull();
  });

  it("maps an empty prompt_path to null on submit", () => {
    render(
      <ProjectEditor
        projectId={"j-eng" as never}
        creator="alice"
        initial={{
          key: "engineering" as never,
          name: "Engineering",
          statuses: [
            {
              key: "open" as never,
              label: "Open",
              color: "#abcdef" as never,
              unblocks_parents: false,
              unblocks_dependents: false,
              cascades_to_children: false,
              on_enter: null,
              prompt_path: "/projects/engineering/statuses/open.md",
            },
          ],
          default_status_key: "open" as never,
          creator: "alice" as never,
          deleted: false,
          prompt_path: "/projects/engineering/prompt.md",
        }}
      />,
    );

    // The two prompt-path inputs come pre-populated from `initial`.
    fireEvent.change(screen.getByTestId(PROMPT_TESTID), {
      target: { value: "" },
    });
    fireEvent.change(screen.getByTestId("status-editor-prompt-path-0"), {
      target: { value: "" },
    });

    fireEvent.click(screen.getByTestId(SAVE_TESTID));

    expect(mutateSpy).toHaveBeenCalledTimes(1);
    const req = mutateSpy.mock.calls[0][0] as {
      project: {
        prompt_path: string | null;
        statuses: { prompt_path: string | null }[];
      };
    };
    expect(req.project.prompt_path).toBeNull();
    expect(req.project.statuses[0]?.prompt_path).toBeNull();
  });

  it("rejects a project prompt_path that does not start with '/'", () => {
    render(<ProjectEditor creator="alice" />);

    fireEvent.change(screen.getByTestId("project-editor-key"), {
      target: { value: "engineering" },
    });
    fireEvent.change(screen.getByTestId("project-editor-name"), {
      target: { value: "Engineering" },
    });
    fireEvent.change(screen.getByTestId(PROMPT_TESTID), {
      target: { value: "not-a-path" },
    });

    // The save button reflects the inline validation error and disables;
    // mutate is never invoked.
    const saveBtn = screen.getByTestId(SAVE_TESTID) as HTMLButtonElement;
    expect(saveBtn.disabled).toBe(true);
    expect(
      screen.getByText(
        "Project prompt path must be a doc-store path starting with '/'",
      ),
    ).toBeDefined();
    expect(mutateSpy).not.toHaveBeenCalled();
  });

  it("rejects a per-status prompt_path that does not start with '/'", () => {
    render(<ProjectEditor creator="alice" />);

    fireEvent.change(screen.getByTestId("project-editor-key"), {
      target: { value: "engineering" },
    });
    fireEvent.change(screen.getByTestId("project-editor-name"), {
      target: { value: "Engineering" },
    });
    fireEvent.change(screen.getByTestId("status-editor-prompt-path-0"), {
      target: { value: "still-no-slash" },
    });

    const saveBtn = screen.getByTestId(SAVE_TESTID) as HTMLButtonElement;
    expect(saveBtn.disabled).toBe(true);
    expect(
      screen.getByText(
        "Status 'open' prompt path must be a doc-store path starting with '/'",
      ),
    ).toBeDefined();
    expect(mutateSpy).not.toHaveBeenCalled();
  });
});
