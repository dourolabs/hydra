// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { ReactNode } from "react";

const mutateSpy = vi.fn();
const addToastSpy = vi.fn();

// The save fan-out tests need to drive `mutationFn` directly so they
// can observe the underlying per-status API spies. ProjectEditor calls
// `useMutation` twice per render (save then delete); we record the
// most recent save-mutation fn here (every even-indexed call).
let capturedSaveMutationFn:
  | ((payload: unknown) => Promise<unknown>)
  | null = null;
let useMutationCallIndex = 0;

vi.mock("react-router-dom", () => ({
  useNavigate: () => vi.fn(),
}));

vi.mock("@tanstack/react-query", () => ({
  useMutation: (opts: { mutationFn: (payload: unknown) => Promise<unknown> }) => {
    if (useMutationCallIndex % 2 === 0) {
      capturedSaveMutationFn = opts.mutationFn;
    }
    useMutationCallIndex += 1;
    return { mutate: mutateSpy, isPending: false };
  },
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

const createProjectSpy = vi.fn(
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  async (_req: unknown) => ({ project_id: "j-test", version: 1 }),
);
const updateProjectSpy = vi.fn(
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  async (_id: string, _req: unknown) => ({ project_id: "j-test", version: 1 }),
);
const createProjectStatusSpy = vi.fn(
  async (_id: string, status: unknown) => ({
    project_id: _id,
    version: 1,
    status,
  }),
);
const updateProjectStatusSpy = vi.fn(
  async (_id: string, _key: string, status: unknown) => ({
    project_id: _id,
    version: 1,
    status,
  }),
);
const deleteProjectStatusSpy = vi.fn(
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  async (_id: string, _key: string) => ({ project_id: _id, version: 1 }),
);

vi.mock("../../../api/client", () => ({
  apiClient: {
    createProject: createProjectSpy,
    updateProject: updateProjectSpy,
    deleteProject: vi.fn(async () => ({ project_id: "j-x", version: 1 })),
    createProjectStatus: createProjectStatusSpy,
    updateProjectStatus: updateProjectStatusSpy,
    deleteProjectStatus: deleteProjectStatusSpy,
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

function resetSpies() {
  addToastSpy.mockReset();
  mutateSpy.mockReset();
  createProjectSpy.mockClear();
  updateProjectSpy.mockClear();
  createProjectStatusSpy.mockClear();
  updateProjectStatusSpy.mockClear();
  deleteProjectStatusSpy.mockClear();
  capturedSaveMutationFn = null;
  useMutationCallIndex = 0;
}

describe("ProjectEditor — prompt_path", () => {
  beforeEach(() => {
    resetSpies();
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
              position: 0,
            },
          ],
          creator: "alice" as never,
          deleted: false,
          prompt_path: "/projects/engineering/prompt.md",
          priority: 0,
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

// Shape-agnostic helper for building an existing status. The cast to
// `never` mirrors the existing tests above — the api-package types are
// nominal at the boundary, but our test props pass through unchanged.
function status(overrides: {
  key: string;
  label?: string;
  prompt_path?: string | null;
  position?: number;
}) {
  return {
    key: overrides.key as never,
    label: overrides.label ?? overrides.key,
    color: "#abcdef" as never,
    unblocks_parents: false,
    unblocks_dependents: false,
    cascades_to_children: false,
    on_enter: null,
    prompt_path: overrides.prompt_path ?? null,
    position: overrides.position ?? 0,
  };
}

function existingProject(statuses: ReturnType<typeof status>[]) {
  return {
    key: "engineering" as never,
    name: "Engineering",
    statuses,
    creator: "alice" as never,
    deleted: false,
    prompt_path: null,
    priority: 0,
  };
}

describe("ProjectEditor — status fan-out diff", () => {
  beforeEach(() => {
    resetSpies();
  });

  afterEach(() => {
    cleanup();
  });

  it("rename: emits a single updateProjectStatus PUT keyed by the original key", async () => {
    render(
      <ProjectEditor
        projectId={"j-eng" as never}
        creator="alice"
        initial={existingProject([
          status({ key: "open", label: "Open" }),
          status({ key: "closed", label: "Closed", position: 100 }),
        ])}
      />,
    );

    fireEvent.change(screen.getByTestId("status-editor-key-0"), {
      target: { value: "todo" },
    });
    fireEvent.click(screen.getByTestId(SAVE_TESTID));

    expect(mutateSpy).toHaveBeenCalledTimes(1);
    const payload = mutateSpy.mock.calls[0][0] as {
      nextStatuses: { key: string }[];
      initialStatuses: { key: string }[];
      originalKeys: (string | null)[];
    };
    // Original snapshot is preserved on the payload — the rename is
    // visible as "next key differs from original key" rather than the
    // old "next key absent from initial set" heuristic.
    expect(payload.nextStatuses[0].key).toBe("todo");
    expect(payload.initialStatuses[0].key).toBe("open");
    expect(payload.originalKeys[0]).toBe("open");

    expect(capturedSaveMutationFn).not.toBeNull();
    await capturedSaveMutationFn!(payload);

    expect(updateProjectStatusSpy).toHaveBeenCalledTimes(2);
    expect(updateProjectStatusSpy).toHaveBeenNthCalledWith(
      1,
      "j-test",
      "open",
      expect.objectContaining({ key: "todo" }),
    );
    expect(createProjectStatusSpy).not.toHaveBeenCalled();
    expect(deleteProjectStatusSpy).not.toHaveBeenCalled();
  });

  it("non-key edit: preserves the original key on the update PUT", async () => {
    render(
      <ProjectEditor
        projectId={"j-eng" as never}
        creator="alice"
        initial={existingProject([status({ key: "open", label: "Open" })])}
      />,
    );

    const labelInput = screen.getByDisplayValue("Open");
    fireEvent.change(labelInput, { target: { value: "Backlog" } });
    fireEvent.click(screen.getByTestId(SAVE_TESTID));

    const payload = mutateSpy.mock.calls[0][0];
    await capturedSaveMutationFn!(payload);

    expect(updateProjectStatusSpy).toHaveBeenCalledTimes(1);
    expect(updateProjectStatusSpy).toHaveBeenCalledWith(
      "j-test",
      "open",
      expect.objectContaining({ key: "open", label: "Backlog" }),
    );
    expect(createProjectStatusSpy).not.toHaveBeenCalled();
    expect(deleteProjectStatusSpy).not.toHaveBeenCalled();
  });

  it("add new row: emits a createProjectStatus POST for the new row", async () => {
    render(<ProjectEditor creator="alice" />);

    fireEvent.change(screen.getByTestId("project-editor-key"), {
      target: { value: "engineering" },
    });
    fireEvent.change(screen.getByTestId("project-editor-name"), {
      target: { value: "Engineering" },
    });

    // New-project flow seeds three rows; clicking + Add appends a
    // fourth whose blankStatus key is `status-4`. Rename it for a
    // distinct assertion target.
    fireEvent.click(screen.getByTestId("project-editor-add-status"));
    fireEvent.change(screen.getByTestId("status-editor-key-3"), {
      target: { value: "review" },
    });

    fireEvent.click(screen.getByTestId(SAVE_TESTID));

    const payload = mutateSpy.mock.calls[0][0];
    await capturedSaveMutationFn!(payload);

    // All four rows were added in this edit (no `initial`), so they all
    // POST and none of them PUT/DELETE.
    expect(createProjectStatusSpy).toHaveBeenCalledTimes(4);
    const newCall = createProjectStatusSpy.mock.calls.find(
      ([, body]) => (body as { key: string }).key === "review",
    );
    expect(newCall).toBeDefined();
    expect(updateProjectStatusSpy).not.toHaveBeenCalled();
    expect(deleteProjectStatusSpy).not.toHaveBeenCalled();
  });

  it("rename after reorder: PUT path-segment is the moved row's original key", async () => {
    render(
      <ProjectEditor
        projectId={"j-eng" as never}
        creator="alice"
        initial={existingProject([
          status({ key: "open", label: "Open", position: 0 }),
          status({ key: "in-progress", label: "In progress", position: 100 }),
          status({ key: "closed", label: "Closed", position: 200 }),
        ])}
      />,
    );

    // Move the closed row (index 2) up so it sits at index 1.
    const moveUpButtons = screen.getAllByLabelText("Move up");
    fireEvent.click(moveUpButtons[2]);

    // After the swap, the closed row's Key input lives at index 1 in
    // the rendered grid (React key changes with index, but the row's
    // originalKey is held in parent state and follows the row).
    fireEvent.change(screen.getByTestId("status-editor-key-1"), {
      target: { value: "done" },
    });

    fireEvent.click(screen.getByTestId(SAVE_TESTID));

    const payload = mutateSpy.mock.calls[0][0] as {
      nextStatuses: { key: string }[];
      originalKeys: (string | null)[];
    };
    // Sanity: the renamed row sits at index 1 (post-reorder) and its
    // tracked original key is `closed`, not `in-progress`.
    expect(payload.nextStatuses[1].key).toBe("done");
    expect(payload.originalKeys[1]).toBe("closed");

    await capturedSaveMutationFn!(payload);

    const renameCall = updateProjectStatusSpy.mock.calls.find(
      ([, , body]) => (body as { key: string }).key === "done",
    );
    expect(renameCall).toBeDefined();
    // Path segment is the row's original key, NOT `in-progress` (the
    // row that swapped past it) and NOT `done` (the renamed value).
    expect(renameCall![1]).toBe("closed");
    expect(createProjectStatusSpy).not.toHaveBeenCalled();
    expect(deleteProjectStatusSpy).not.toHaveBeenCalled();
  });
});
