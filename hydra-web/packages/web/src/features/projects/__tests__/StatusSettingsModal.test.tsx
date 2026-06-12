// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import type {
  ListProjectsResponse,
  ProjectRecord,
  StatusDefinition,
} from "@hydra/api";
import type { ReactNode } from "react";

const mutateSpy = vi.fn();
const addToastSpy = vi.fn();
let mutationPending = false;
let simulateError: Error | null = null;
const cancelQueriesSpy = vi.fn(async () => {});
const setQueryDataSpy = vi.fn();
const invalidateQueriesSpy = vi.fn();
let queryDataByKey: Map<string, unknown> = new Map();

type QueryState = {
  isLoading: boolean;
  isError: boolean;
  isSuccess: boolean;
  data: unknown;
  error: Error | null;
};
let promptQueryState: QueryState = {
  isLoading: false,
  isError: false,
  isSuccess: true,
  data: null,
  error: null,
};
const lastPromptQueryKeyRef: { key: readonly unknown[] | null } = { key: null };
const lastPromptQueryFnRef: { fn: (() => unknown) | null } = { fn: null };

vi.mock("@tanstack/react-query", () => ({
  useQuery: ({
    queryKey,
    queryFn,
  }: {
    queryKey: readonly unknown[];
    queryFn: () => unknown;
  }) => {
    lastPromptQueryKeyRef.key = queryKey;
    lastPromptQueryFnRef.fn = queryFn;
    return promptQueryState;
  },
  useMutation: ({
    mutationFn,
    onMutate,
    onSuccess,
    onError,
  }: {
    mutationFn?: (vars: unknown) => Promise<{ project_id: string }>;
    onMutate?: (vars: unknown) => Promise<unknown> | unknown;
    onSuccess?: (response: { project_id: string }, vars: unknown) => void;
    onError?: (err: Error, vars: unknown, context: unknown) => void;
  }) => ({
    mutate: (
      vars: unknown,
      perCall?: {
        onSuccess?: (response: { project_id: string }, vars: unknown) => void;
        onError?: (err: Error, vars: unknown, context: unknown) => void;
      },
    ) => {
      mutateSpy(vars);
      // Mirror react-query: await onMutate (so its inner awaits resolve)
      // before either firing the success path or invoking onError. If no
      // onMutate (e.g. moveAndDelete mutation), drive the success/error
      // path off the mutationFn promise instead so the bulk move's per-
      // issue API loop actually runs before onSuccess fires.
      void (async () => {
        if (onMutate) {
          const ctx = await Promise.resolve(onMutate(vars));
          if (simulateError) {
            onError?.(simulateError, vars, ctx);
            perCall?.onError?.(simulateError, vars, ctx);
            return;
          }
          void mutationFn?.(vars);
          const response = { project_id: "j-eng" };
          onSuccess?.(response, vars);
          perCall?.onSuccess?.(response, vars);
          return;
        }
        try {
          const response = (await mutationFn?.(vars)) ?? {
            project_id: "j-eng",
          };
          onSuccess?.(response, vars);
          perCall?.onSuccess?.(response, vars);
        } catch (err) {
          onError?.(err as Error, vars, undefined);
          perCall?.onError?.(err as Error, vars, undefined);
        }
      })();
    },
    isPending: mutationPending,
  }),
  useQueryClient: () => ({
    cancelQueries: cancelQueriesSpy,
    getQueryData: (key: readonly unknown[]) =>
      queryDataByKey.get(JSON.stringify(key)),
    setQueryData: (key: readonly unknown[], value: unknown) => {
      queryDataByKey.set(JSON.stringify(key), value);
      setQueryDataSpy(key, value);
    },
    invalidateQueries: invalidateQueriesSpy,
  }),
}));

vi.mock("@hydra/ui", () => ({
  Button: ({
    children,
    onClick,
    disabled,
    title,
    "data-testid": testId,
  }: {
    children: ReactNode;
    onClick?: () => void;
    disabled?: boolean;
    title?: string;
    variant?: string;
    size?: string;
    "data-testid"?: string;
  }) => (
    <button
      onClick={onClick}
      disabled={disabled}
      title={title}
      data-testid={testId}
    >
      {children}
    </button>
  ),
  Input: ({
    label,
    value,
    onChange,
    placeholder,
    disabled,
    type,
    "aria-label": ariaLabel,
    "data-testid": testId,
  }: {
    label?: string;
    value: string;
    onChange?: (e: { target: { value: string } }) => void;
    placeholder?: string;
    required?: boolean;
    disabled?: boolean;
    type?: string;
    min?: number;
    id?: string;
    "aria-label"?: string;
    "data-testid"?: string;
  }) => (
    <label>
      {label}
      <input
        type={type}
        value={value}
        disabled={disabled}
        onChange={(e) =>
          onChange?.({ target: { value: e.target.value } })
        }
        placeholder={placeholder}
        aria-label={ariaLabel}
        data-testid={testId}
      />
    </label>
  ),
  Modal: ({
    open,
    children,
    title,
  }: {
    open: boolean;
    onClose?: () => void;
    title?: string;
    children: ReactNode;
  }) =>
    open ? (
      <div role="dialog" aria-label={title} data-testid="status-settings-modal">
        <h2>{title}</h2>
        {children}
      </div>
    ) : null,
  Select: ({
    label,
    options,
    value,
    onChange,
    "aria-label": ariaLabel,
    "data-testid": testId,
  }: {
    label?: string;
    options: { value: string; label: string }[];
    value: string;
    onChange: (e: { target: { value: string } }) => void;
    "aria-label"?: string;
    "data-testid"?: string;
  }) => (
    <label>
      {label}
      <select
        value={value}
        onChange={(e) =>
          onChange({ target: { value: e.target.value } })
        }
        aria-label={ariaLabel}
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
  Textarea: ({
    label,
    value,
    onChange,
    placeholder,
    "data-testid": testId,
  }: {
    label?: string;
    value: string;
    onChange?: (e: { target: { value: string } }) => void;
    placeholder?: string;
    rows?: number;
    "data-testid"?: string;
  }) => (
    <label>
      {label}
      <textarea
        value={value}
        onChange={(e) =>
          onChange?.({ target: { value: e.target.value } })
        }
        placeholder={placeholder}
        data-testid={testId}
      />
    </label>
  ),
  Avatar: ({ name }: { name: string; kind?: string; size?: string }) => (
    <span data-testid={`avatar-${name}`}>{name}</span>
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
    wide?: boolean;
    children: ReactNode;
  }) => (
    <div>
      <div>{label}</div>
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={open}
        aria-label={label}
      >
        {value}
      </button>
      {open && <div role="menu">{children}</div>}
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
      role="menuitem"
      aria-pressed={!!active}
      onClick={onClick}
    >
      {children}
    </button>
  ),
  ColorPicker: ({ value }: { value: string }) => (
    <span data-testid={`color-${value}`}>{value}</span>
  ),
}));

const updateProjectSpy = vi.fn(async (_id: string, req: unknown) => req);
const createProjectStatusSpy = vi.fn(
  async (
    _id: string,
    status: unknown,
  ): Promise<{ project_id: string; version: number; status: unknown }> => ({
    project_id: _id,
    version: 1,
    status,
  }),
);
const updateProjectStatusSpy = vi.fn(
  async (
    _id: string,
    _key: string,
    status: unknown,
  ): Promise<{ project_id: string; version: number; status: unknown }> => ({
    project_id: _id,
    version: 1,
    status,
  }),
);
const deleteProjectStatusSpy = vi.fn(
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  async (_id: string, _key: string): Promise<{ project_id: string; version: number }> => ({
    project_id: _id,
    version: 1,
  }),
);
type ListIssuesMockResp = {
  issues: Array<{ issue_id: string }>;
  next_cursor: string | null;
};
const listIssuesSpy = vi.fn(
  async (
    query: Record<string, unknown>,
  ): Promise<ListIssuesMockResp> => {
    void query;
    return { issues: [], next_cursor: null };
  },
);
const getIssueSpy = vi.fn(async (id: string) => ({
  issue_id: id,
  version: 1,
  timestamp: "2026-01-01T00:00:00Z",
  creation_time: "2026-01-01T00:00:00Z",
  issue: {
    type: "task",
    title: id,
    description: "full description",
    creator: "alice",
    progress: "",
    status: "in-progress",
    project_id: "j-eng",
    assignee: null,
    dependencies: [],
    patches: [],
  },
}));
const updateIssueSpy = vi.fn(async (_id: string, req: unknown) => req);
const getDocumentByPathSpy = vi.fn();
const createDocumentSpy = vi.fn();
const updateDocumentSpy = vi.fn();
class ApiErrorMock extends Error {
  constructor(public readonly status: number, message: string) {
    super(message);
    this.name = "ApiError";
  }
}
vi.mock("../../../api/client", () => ({
  ApiError: ApiErrorMock,
  apiClient: {
    updateProject: updateProjectSpy,
    createProjectStatus: createProjectStatusSpy,
    updateProjectStatus: updateProjectStatusSpy,
    deleteProjectStatus: deleteProjectStatusSpy,
    listIssues: listIssuesSpy,
    getIssue: getIssueSpy,
    updateIssue: updateIssueSpy,
    getDocumentByPath: getDocumentByPathSpy,
    createDocument: createDocumentSpy,
    updateDocument: updateDocumentSpy,
  },
}));

vi.mock("../../toast/useToast", () => ({
  useToast: () => ({ addToast: addToastSpy }),
}));

let agentsList: { name: string }[] = [];
vi.mock("../../../hooks/useAgents", () => ({
  useAgents: () => ({ data: agentsList }),
}));

let usersList: { username: string }[] = [];
vi.mock("../../../hooks/useUsers", () => ({
  useUsers: () => ({ data: usersList }),
}));

vi.mock("../../../components/ColorPicker", () => ({
  LABEL_COLOR_PALETTE: ["#111", "#222", "#333", "#444", "#555", "#666"],
}));

vi.mock("../StatusSettingsModal.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("../PromptDocumentEditor.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const { StatusSettingsModal } = await import("../StatusSettingsModal");

function makeStatus(
  key: string,
  overrides: Partial<StatusDefinition> = {},
): StatusDefinition {
  return {
    key: key as never,
    label: key,
    color: "#abcdef" as never,
    position: 0,
    unblocks_parents: false,
    unblocks_dependents: false,
    cascades_to_children: false,
    on_enter: null,
    prompt_path: null,
    ...overrides,
  };
}

function makeProject(statuses: StatusDefinition[]): ProjectRecord {
  return {
    project_id: "j-eng" as never,
    version: 1,
    project: {
      key: "engineering" as never,
      name: "Engineering",
      statuses,
      creator: "alice" as never,
      deleted: false,
      prompt_path: null,
      priority: 0,
    },
  };
}

describe("StatusSettingsModal", () => {
  beforeEach(() => {
    mutateSpy.mockReset();
    addToastSpy.mockReset();
    updateProjectSpy.mockClear();
    createProjectStatusSpy.mockClear();
    updateProjectStatusSpy.mockClear();
    deleteProjectStatusSpy.mockClear();
    listIssuesSpy.mockReset();
    listIssuesSpy.mockImplementation(async () => ({
      issues: [],
      next_cursor: null,
    }));
    getIssueSpy.mockClear();
    updateIssueSpy.mockClear();
    cancelQueriesSpy.mockClear();
    setQueryDataSpy.mockClear();
    invalidateQueriesSpy.mockClear();
    getDocumentByPathSpy.mockReset();
    createDocumentSpy.mockReset();
    updateDocumentSpy.mockReset();
    queryDataByKey = new Map();
    mutationPending = false;
    simulateError = null;
    agentsList = [];
    usersList = [];
    promptQueryState = {
      isLoading: false,
      isError: false,
      isSuccess: true,
      data: null,
      error: null,
    };
    lastPromptQueryKeyRef.key = null;
    lastPromptQueryFnRef.fn = null;
  });

  afterEach(() => {
    cleanup();
  });

  it("opens pre-filled with the requested status's state", () => {
    const project = makeProject([
      makeStatus("open", { label: "Open" }),
      makeStatus("in-progress", { label: "In progress" }),
    ]);
    render(
      <StatusSettingsModal
        open={true}
        onClose={() => {}}
        projectRecord={project}
        statusKey="in-progress"
        issueCount={0}
      />,
    );

    const label = screen.getByTestId("status-settings-label") as HTMLInputElement;
    expect(label.value).toBe("In progress");
    // The key is no longer shown — it is derived from the name on save.
    expect(screen.queryByTestId("status-settings-key")).toBeNull();
  });

  it("Save fires updateProject with the modified status, re-deriving the key from the name", () => {
    const project = makeProject([
      makeStatus("open", { label: "Open" }),
      makeStatus("in-progress", { label: "In progress" }),
    ]);
    render(
      <StatusSettingsModal
        open={true}
        onClose={() => {}}
        projectRecord={project}
        statusKey="in-progress"
        issueCount={0}
      />,
    );

    fireEvent.change(screen.getByTestId("status-settings-label"), {
      target: { value: "Doing" },
    });
    fireEvent.click(screen.getByTestId("status-settings-save"));

    expect(mutateSpy).toHaveBeenCalledTimes(1);
    const payload = mutateSpy.mock.calls[0][0] as {
      nextStatuses: StatusDefinition[];
      action: "edit" | "delete";
    };
    expect(payload.action).toBe("edit");
    const next = payload.nextStatuses;
    expect(next).toHaveLength(2);
    expect(next[0].key).toBe("open");
    // Key follows the renamed name: "Doing" → "doing".
    expect(next[1].key).toBe("doing");
    expect(next[1].label).toBe("Doing");
  });

  it("disables Save and surfaces an error when the renamed key collides with a sibling", () => {
    const project = makeProject([
      makeStatus("open", { label: "Open" }),
      makeStatus("in-progress", { label: "In progress" }),
    ]);
    render(
      <StatusSettingsModal
        open={true}
        onClose={() => {}}
        projectRecord={project}
        statusKey="in-progress"
        issueCount={0}
      />,
    );

    // Rename "In progress" to collide with the sibling "open" status.
    fireEvent.change(screen.getByTestId("status-settings-label"), {
      target: { value: "Open" },
    });

    const save = screen.getByTestId("status-settings-save") as HTMLButtonElement;
    expect(save.disabled).toBe(true);
    expect(
      screen.getByTestId("status-settings-new-error").textContent,
    ).toContain("already exists");
  });

  it("does not render move-left / move-right buttons (drag-and-drop owns reordering)", () => {
    const project = makeProject([
      makeStatus("open"),
      makeStatus("in-progress"),
      makeStatus("closed"),
    ]);
    render(
      <StatusSettingsModal
        open={true}
        onClose={() => {}}
        projectRecord={project}
        statusKey="in-progress"
        issueCount={0}
      />,
    );
    expect(screen.queryByTestId("status-settings-move-left")).toBeNull();
    expect(screen.queryByTestId("status-settings-move-right")).toBeNull();
  });

  it("Delete on a non-empty column reveals the Move sub-step with a neighbor default", () => {
    const project = makeProject([
      makeStatus("open"),
      makeStatus("in-progress"),
      makeStatus("closed"),
    ]);
    render(
      <StatusSettingsModal
        open={true}
        onClose={() => {}}
        projectRecord={project}
        statusKey="in-progress"
        issueCount={3}
      />,
    );

    const del = screen.getByTestId("status-settings-delete") as HTMLButtonElement;
    // PR-12: Delete is no longer disabled — it opens the Move sub-step.
    expect(del.disabled).toBe(false);
    fireEvent.click(del);

    expect(screen.getByTestId("status-settings-move-block")).toBeDefined();
    const select = screen.getByTestId(
      "status-settings-move-target",
    ) as HTMLSelectElement;
    // Default neighbor for deleting "in-progress" is the left one ("open").
    expect(select.value).toBe("open");
    // Option list excludes the to-delete status.
    const values = Array.from(select.options).map((o) => o.value);
    expect(values).toEqual(["open", "closed"]);

    const confirm = screen.getByTestId(
      "status-settings-move-confirm",
    ) as HTMLButtonElement;
    expect(confirm.textContent).toBe("Move 3 and delete");
  });

  it("Move sub-step defaults to the right neighbor when deleting the leftmost status", () => {
    const project = makeProject([
      makeStatus("open"),
      makeStatus("in-progress"),
      makeStatus("closed"),
    ]);
    render(
      <StatusSettingsModal
        open={true}
        onClose={() => {}}
        projectRecord={project}
        statusKey="open"
        issueCount={2}
      />,
    );

    fireEvent.click(screen.getByTestId("status-settings-delete"));
    const select = screen.getByTestId(
      "status-settings-move-target",
    ) as HTMLSelectElement;
    expect(select.value).toBe("in-progress");
  });

  it("Delete is disabled when the project has only one status", () => {
    const project = makeProject([makeStatus("open")]);
    render(
      <StatusSettingsModal
        open={true}
        onClose={() => {}}
        projectRecord={project}
        statusKey="open"
        issueCount={0}
      />,
    );

    const del = screen.getByTestId("status-settings-delete") as HTMLButtonElement;
    expect(del.disabled).toBe(true);
    expect(del.title).toBe("Cannot delete the only status");
  });

  it("Delete with confirm removes the status from the project array and persists", () => {
    const project = makeProject([
      makeStatus("open"),
      makeStatus("in-progress"),
      makeStatus("closed"),
    ]);
    render(
      <StatusSettingsModal
        open={true}
        onClose={() => {}}
        projectRecord={project}
        statusKey="in-progress"
        issueCount={0}
      />,
    );

    fireEvent.click(screen.getByTestId("status-settings-delete"));
    fireEvent.click(screen.getByTestId("status-settings-delete-confirm"));

    expect(mutateSpy).toHaveBeenCalledTimes(1);
    const payload = mutateSpy.mock.calls[0][0] as {
      nextStatuses: StatusDefinition[];
      action: "edit" | "delete";
    };
    expect(payload.action).toBe("delete");
    expect(payload.nextStatuses.map((s) => s.key)).toEqual(["open", "closed"]);
  });

  it("Save applies an optimistic update to the projects cache", async () => {
    const project = makeProject([
      makeStatus("open", { label: "Open" }),
      makeStatus("in-progress", { label: "In progress" }),
    ]);
    queryDataByKey.set(
      JSON.stringify(["projects"]),
      { projects: [project] } as ListProjectsResponse,
    );
    render(
      <StatusSettingsModal
        open={true}
        onClose={() => {}}
        projectRecord={project}
        statusKey="in-progress"
        issueCount={0}
      />,
    );

    fireEvent.change(screen.getByTestId("status-settings-label"), {
      target: { value: "Doing" },
    });
    await act(async () => {
      fireEvent.click(screen.getByTestId("status-settings-save"));
    });

    expect(cancelQueriesSpy).toHaveBeenCalled();
    expect(setQueryDataSpy).toHaveBeenCalled();
    const lastSet = setQueryDataSpy.mock.calls.find(
      (call) => JSON.stringify(call[0]) === JSON.stringify(["projects"]),
    );
    expect(lastSet).toBeDefined();
    const cached = lastSet![1] as ListProjectsResponse;
    expect(cached.projects).toHaveLength(1);
    expect(cached.projects[0].project.statuses[1].label).toBe("Doing");
    expect(cached.projects[0].version).toBe(2);
  });

  it("rolls back the projects cache when the save mutation errors", async () => {
    const project = makeProject([
      makeStatus("open"),
      makeStatus("in-progress"),
    ]);
    const previous = { projects: [project] } as ListProjectsResponse;
    queryDataByKey.set(JSON.stringify(["projects"]), previous);
    simulateError = new Error("boom");

    render(
      <StatusSettingsModal
        open={true}
        onClose={() => {}}
        projectRecord={project}
        statusKey="in-progress"
        issueCount={0}
      />,
    );

    await act(async () => {
      fireEvent.click(screen.getByTestId("status-settings-save"));
    });

    // The last setQueryData for ["projects"] should restore the snapshot.
    const projectsCalls = setQueryDataSpy.mock.calls.filter(
      (call) => JSON.stringify(call[0]) === JSON.stringify(["projects"]),
    );
    expect(projectsCalls.length).toBeGreaterThanOrEqual(2);
    expect(projectsCalls[projectsCalls.length - 1][1]).toBe(previous);
    expect(addToastSpy).toHaveBeenCalledWith("boom", "error");
  });

  it("Save closes the modal after a successful mutation", async () => {
    const project = makeProject([
      makeStatus("open"),
      makeStatus("in-progress"),
    ]);
    const onClose = vi.fn();
    render(
      <StatusSettingsModal
        open={true}
        onClose={onClose}
        projectRecord={project}
        statusKey="open"
        issueCount={0}
      />,
    );

    await act(async () => {
      fireEvent.click(screen.getByTestId("status-settings-save"));
    });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  describe("new mode", () => {
    it("renders the same rich form as edit — Name, assignee picker, prompt editor; no Key field", () => {
      const project = makeProject([
        makeStatus("open"),
        makeStatus("in-progress"),
      ]);
      render(
        <StatusSettingsModal
          open={true}
          mode="new"
          onClose={() => {}}
          projectRecord={project}
        />,
      );

      // The Name field and advanced controls mirror the edit modal.
      expect(screen.getByTestId("status-settings-label")).toBeDefined();
      expect(screen.getByTestId("status-settings-assignee")).toBeDefined();
      // Prompt is the same inline markdown editor as edit mode, always visible.
      expect(screen.getByTestId("status-settings-prompt-body")).toBeDefined();
      // The resolved save path is shown (no editable path field, no toggle).
      expect(screen.queryByTestId("status-settings-prompt-path-toggle")).toBeNull();
      // The key is derived from the name, never shown as its own field.
      expect(screen.queryByTestId("status-settings-key")).toBeNull();
    });

    it("hides the delete control in new mode", () => {
      const project = makeProject([
        makeStatus("open"),
        makeStatus("in-progress"),
      ]);
      render(
        <StatusSettingsModal
          open={true}
          mode="new"
          onClose={() => {}}
          projectRecord={project}
        />,
      );

      expect(screen.queryByTestId("status-settings-delete")).toBeNull();
    });

    it("Save derives the key from the name and appends the status", async () => {
      const project = makeProject([
        makeStatus("open", { label: "Open" }),
        makeStatus("in-progress", { label: "In progress" }),
      ]);
      render(
        <StatusSettingsModal
          open={true}
          mode="new"
          onClose={() => {}}
          projectRecord={project}
        />,
      );

      fireEvent.change(screen.getByTestId("status-settings-label"), {
        target: { value: "In Review" },
      });
      await act(async () => {
        fireEvent.click(screen.getByTestId("status-settings-save"));
      });

      expect(createProjectStatusSpy).toHaveBeenCalledTimes(1);
      const [postedProjectId, postedStatus] =
        createProjectStatusSpy.mock.calls[0] as unknown as [
          string,
          StatusDefinition,
        ];
      expect(postedProjectId).toBe("j-eng");
      expect(postedStatus.key).toBe("in-review");
      expect(postedStatus.label).toBe("In Review");
      // The status points at the derived auto path.
      expect(postedStatus.prompt_path).toBe(
        "/projects/engineering/statuses/in-review.md",
      );
    });

    it("shows the resolved save path, filling in the name's slug", () => {
      const project = makeProject([makeStatus("open")]);
      render(
        <StatusSettingsModal
          open={true}
          mode="new"
          onClose={() => {}}
          projectRecord={project}
        />,
      );

      fireEvent.change(screen.getByTestId("status-settings-label"), {
        target: { value: "In Review" },
      });
      expect(
        screen.getByTestId("status-settings-prompt-path").textContent,
      ).toContain("/projects/engineering/statuses/in-review.md");
    });

    it("writes the prompt document at the derived path on save", async () => {
      const project = makeProject([makeStatus("open")]);
      // No existing doc at the derived path → 404 → createDocument fires.
      getDocumentByPathSpy.mockImplementation(async () => {
        throw new ApiErrorMock(404, "not found");
      });
      createDocumentSpy.mockImplementation(async () => ({ document_id: "d-new" }));

      render(
        <StatusSettingsModal
          open={true}
          mode="new"
          onClose={() => {}}
          projectRecord={project}
        />,
      );

      fireEvent.change(screen.getByTestId("status-settings-label"), {
        target: { value: "Blocked" },
      });
      fireEvent.change(screen.getByTestId("status-settings-prompt-body"), {
        target: { value: "Body text" },
      });
      await act(async () => {
        fireEvent.click(screen.getByTestId("status-settings-save"));
      });

      expect(getDocumentByPathSpy).toHaveBeenCalledWith(
        "/projects/engineering/statuses/blocked.md",
      );
      expect(createDocumentSpy).toHaveBeenCalledTimes(1);
      const docPayload = createDocumentSpy.mock.calls[0][0] as {
        document: { path: string; body_markdown: string };
      };
      expect(docPayload.document.path).toBe(
        "/projects/engineering/statuses/blocked.md",
      );
      expect(docPayload.document.body_markdown).toBe("Body text");
      expect(createProjectStatusSpy).toHaveBeenCalledTimes(1);
    });

    it("skips the document write when the prompt body is empty", async () => {
      const project = makeProject([makeStatus("open")]);
      render(
        <StatusSettingsModal
          open={true}
          mode="new"
          onClose={() => {}}
          projectRecord={project}
        />,
      );
      fireEvent.change(screen.getByTestId("status-settings-label"), {
        target: { value: "Blocked" },
      });
      await act(async () => {
        fireEvent.click(screen.getByTestId("status-settings-save"));
      });

      expect(getDocumentByPathSpy).not.toHaveBeenCalled();
      expect(createDocumentSpy).not.toHaveBeenCalled();
      expect(updateDocumentSpy).not.toHaveBeenCalled();
      // The status still records the derived path so a later edit finds the doc.
      expect(createProjectStatusSpy).toHaveBeenCalledTimes(1);
      const status = createProjectStatusSpy.mock.calls[0][1] as StatusDefinition;
      expect(status.prompt_path).toBe(
        "/projects/engineering/statuses/blocked.md",
      );
    });

    it("disables Save and shows an error when the derived key collides", () => {
      const project = makeProject([
        makeStatus("open"),
        makeStatus("in-progress"),
      ]);
      render(
        <StatusSettingsModal
          open={true}
          mode="new"
          onClose={() => {}}
          projectRecord={project}
        />,
      );

      fireEvent.change(screen.getByTestId("status-settings-label"), {
        target: { value: "Open" },
      });

      const save = screen.getByTestId("status-settings-save") as HTMLButtonElement;
      expect(save.disabled).toBe(true);
      expect(
        screen.getByTestId("status-settings-new-error").textContent,
      ).toContain("already exists");
    });

    it("disables Save when the name has no slug-able characters", () => {
      const project = makeProject([makeStatus("open")]);
      render(
        <StatusSettingsModal
          open={true}
          mode="new"
          onClose={() => {}}
          projectRecord={project}
        />,
      );

      fireEvent.change(screen.getByTestId("status-settings-label"), {
        target: { value: "@@@" },
      });
      const save = screen.getByTestId("status-settings-save") as HTMLButtonElement;
      expect(save.disabled).toBe(true);
      expect(
        screen.getByTestId("status-settings-new-error").textContent,
      ).toContain("letter or digit");
    });

    it("disables Save when the name is blank", () => {
      const project = makeProject([makeStatus("open")]);
      render(
        <StatusSettingsModal
          open={true}
          mode="new"
          onClose={() => {}}
          projectRecord={project}
        />,
      );

      const save = screen.getByTestId("status-settings-save") as HTMLButtonElement;
      expect(save.disabled).toBe(true);
    });

    it("closes after a successful save", async () => {
      const project = makeProject([makeStatus("open")]);
      const onClose = vi.fn();
      render(
        <StatusSettingsModal
          open={true}
          mode="new"
          onClose={onClose}
          projectRecord={project}
        />,
      );

      fireEvent.change(screen.getByTestId("status-settings-label"), {
        target: { value: "Blocked" },
      });
      await act(async () => {
        fireEvent.click(screen.getByTestId("status-settings-save"));
      });
      expect(onClose).toHaveBeenCalledTimes(1);
    });
  });

  describe("Move-and-delete (PR-12)", () => {
    it("moves every issue at the to-delete status, then drops the status", async () => {
      const project = makeProject([
        makeStatus("open"),
        makeStatus("in-progress"),
        makeStatus("closed"),
      ]);
      const onClose = vi.fn();
      // Two pages of listIssues to exercise cursor iteration.
      listIssuesSpy.mockImplementationOnce(async () => ({
        issues: [{ issue_id: "i-aaa" }, { issue_id: "i-bbb" }],
        next_cursor: "page-2",
      }));
      listIssuesSpy.mockImplementationOnce(async () => ({
        issues: [{ issue_id: "i-ccc" }],
        next_cursor: null,
      }));

      render(
        <StatusSettingsModal
          open={true}
          onClose={onClose}
          projectRecord={project}
          statusKey="in-progress"
          issueCount={3}
        />,
      );

      fireEvent.click(screen.getByTestId("status-settings-delete"));
      // Confirm with the default neighbor ("open").
      await act(async () => {
        fireEvent.click(screen.getByTestId("status-settings-move-confirm"));
      });

      expect(listIssuesSpy).toHaveBeenCalledTimes(2);
      expect(listIssuesSpy.mock.calls[0][0]).toMatchObject({
        project_id: "j-eng",
        status: "in-progress",
        limit: null,
      });
      expect(listIssuesSpy.mock.calls[1][0]).toMatchObject({
        cursor: "page-2",
      });

      // Every issue's full body is fetched, then patched to "open".
      expect(getIssueSpy.mock.calls.map((c) => c[0])).toEqual([
        "i-aaa",
        "i-bbb",
        "i-ccc",
      ]);
      expect(updateIssueSpy).toHaveBeenCalledTimes(3);
      const firstPatch = updateIssueSpy.mock.calls[0];
      expect(firstPatch[0]).toBe("i-aaa");
      expect(
        (firstPatch[1] as { issue: { status: string } }).issue.status,
      ).toBe("open");
      // Description is preserved — sourced from the full getIssue body, not
      // the truncated summary.
      expect(
        (firstPatch[1] as { issue: { description: string } }).issue.description,
      ).toBe("full description");

      // After all issues moved, the per-status DELETE fires.
      expect(deleteProjectStatusSpy).toHaveBeenCalledTimes(1);
      expect(deleteProjectStatusSpy.mock.calls[0][0]).toBe("j-eng");
      expect(deleteProjectStatusSpy.mock.calls[0][1]).toBe("in-progress");
      expect(onClose).toHaveBeenCalledTimes(1);
    });

    it("halts the move on a per-issue error and does NOT save the project", async () => {
      const project = makeProject([
        makeStatus("open"),
        makeStatus("in-progress"),
        makeStatus("closed"),
      ]);
      const onClose = vi.fn();
      listIssuesSpy.mockImplementationOnce(async () => ({
        issues: [{ issue_id: "i-good" }, { issue_id: "i-bad" }, { issue_id: "i-third" }],
        next_cursor: null,
      }));
      updateIssueSpy.mockImplementationOnce(async (_id: string, req: unknown) => req);
      updateIssueSpy.mockImplementationOnce(async () => {
        throw new Error("server fell over");
      });

      render(
        <StatusSettingsModal
          open={true}
          onClose={onClose}
          projectRecord={project}
          statusKey="in-progress"
          issueCount={3}
        />,
      );

      fireEvent.click(screen.getByTestId("status-settings-delete"));
      await act(async () => {
        fireEvent.click(screen.getByTestId("status-settings-move-confirm"));
      });

      // i-good succeeded, i-bad failed → halts before i-third.
      expect(updateIssueSpy).toHaveBeenCalledTimes(2);
      expect(updateProjectSpy).not.toHaveBeenCalled();
      expect(onClose).not.toHaveBeenCalled();
      // Toast names the failed issue.
      const failureCall = addToastSpy.mock.calls.find((c) =>
        String(c[0]).includes("i-bad"),
      );
      expect(failureCall).toBeDefined();
      expect(String(failureCall![0])).toContain("server fell over");
      expect(failureCall![1]).toBe("error");
    });

  });

  describe("inline prompt editor", () => {
    it("renders the markdown editor inline (always visible) with the resolved path", () => {
      const project = makeProject([
        makeStatus("open"),
        makeStatus("in-progress"),
      ]);
      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="in-progress"
          issueCount={0}
        />,
      );
      // Textarea is mounted directly — no toggle, no editable path Input.
      expect(screen.getByTestId("status-settings-prompt-body")).toBeDefined();
      expect(
        screen.queryByTestId("status-settings-prompt-path-toggle"),
      ).toBeNull();
      // The resolved save path is shown read-only.
      expect(
        screen.getByTestId("status-settings-prompt-path").textContent,
      ).toContain("/projects/engineering/statuses/in-progress.md");
    });

    it("loads the prompt document at the status's path by default", () => {
      const project = makeProject([
        makeStatus("open"),
        makeStatus("in-progress"),
      ]);
      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="in-progress"
          issueCount={0}
        />,
      );
      // The editor seeds itself from a useQuery against the default path since
      // the status has no explicit prompt_path set.
      expect(lastPromptQueryKeyRef.key).toEqual([
        "documentByPath",
        "/projects/engineering/statuses/in-progress.md",
      ]);
    });

    it("loads from the status's existing prompt_path when one is set", () => {
      const project = makeProject([
        makeStatus("open"),
        makeStatus("in-progress", {
          prompt_path: "/custom/status.md" as never,
        }),
      ]);
      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="in-progress"
          issueCount={0}
        />,
      );
      expect(lastPromptQueryKeyRef.key).toEqual([
        "documentByPath",
        "/custom/status.md",
      ]);
    });

    it("seeds the textarea with the loaded document body", () => {
      promptQueryState = {
        isLoading: false,
        isError: false,
        isSuccess: true,
        data: { document: { body_markdown: "loaded body" } },
        error: null,
      };
      const project = makeProject([makeStatus("in-progress")]);
      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="in-progress"
          issueCount={0}
        />,
      );
      const body = screen.getByTestId(
        "status-settings-prompt-body",
      ) as HTMLTextAreaElement;
      expect(body.value).toBe("loaded body");
    });

    it("Save writes the edited prompt to the derived path", async () => {
      const project = makeProject([
        makeStatus("open"),
        makeStatus("in-progress", { label: "In progress" }),
      ]);
      getDocumentByPathSpy.mockImplementation(async () => ({
        document_id: "d-existing",
        version: 2,
        timestamp: "2026-01-01T00:00:00Z",
        document: {
          title: "t",
          body_markdown: "old",
          path: "/projects/engineering/statuses/in-progress.md",
        },
      }));
      updateDocumentSpy.mockImplementation(async () => ({
        document_id: "d-existing",
      }));

      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="in-progress"
          issueCount={0}
        />,
      );

      fireEvent.change(screen.getByTestId("status-settings-prompt-body"), {
        target: { value: "fresh prompt" },
      });
      await act(async () => {
        fireEvent.click(screen.getByTestId("status-settings-save"));
      });

      expect(updateDocumentSpy).toHaveBeenCalledTimes(1);
      const docCall = updateDocumentSpy.mock.calls[0];
      expect(docCall[0]).toBe("d-existing");
      expect(
        (docCall[1] as { document: { body_markdown: string; path: string } })
          .document,
      ).toMatchObject({
        body_markdown: "fresh prompt",
        path: "/projects/engineering/statuses/in-progress.md",
      });
      // Project save still fires, recording the derived prompt_path.
      const payload = mutateSpy.mock.calls[0][0] as {
        nextStatuses: StatusDefinition[];
      };
      expect(payload.nextStatuses[1].prompt_path).toBe(
        "/projects/engineering/statuses/in-progress.md",
      );
    });
  });

  describe("auto-archive after", () => {
    it("renders empty value + days unit when auto_archive_after_seconds is null", () => {
      const project = makeProject([
        makeStatus("open"),
        makeStatus("in-progress"),
      ]);
      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="in-progress"
          issueCount={0}
        />,
      );
      const value = screen.getByTestId(
        "status-settings-auto-archive-value",
      ) as HTMLInputElement;
      const unit = screen.getByTestId(
        "status-settings-auto-archive-unit",
      ) as HTMLSelectElement;
      expect(value.value).toBe("");
      expect(unit.value).toBe("days");
    });

    // The inverse-rendering rule prefers the largest whole-unit divisor —
    // weeks > days > hours. 1209600 is both 14 days and 2 weeks; the rule
    // picks weeks so a round-tripped value doesn't bloat into "336 hours".
    it.each([
      ["weeks", 604800, "1"],
      ["weeks", 1209600, "2"],
      ["days", 1036800, "12"],
      ["hours", 3600, "1"],
    ])(
      "renders %s when seconds is %i",
      (expectedUnit, seconds, expectedValue) => {
        const project = makeProject([
          makeStatus("in-progress", {
            auto_archive_after_seconds: seconds as unknown as bigint,
          }),
        ]);
        render(
          <StatusSettingsModal
            open={true}
            onClose={() => {}}
            projectRecord={project}
            statusKey="in-progress"
            issueCount={0}
          />,
        );
        const value = screen.getByTestId(
          "status-settings-auto-archive-value",
        ) as HTMLInputElement;
        const unit = screen.getByTestId(
          "status-settings-auto-archive-unit",
        ) as HTMLSelectElement;
        expect(value.value).toBe(expectedValue);
        expect(unit.value).toBe(expectedUnit);
      },
    );

    it("Save persists the new value × unit in seconds", () => {
      const project = makeProject([
        makeStatus("in-progress", { label: "In progress" }),
      ]);
      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="in-progress"
          issueCount={0}
        />,
      );

      fireEvent.change(screen.getByTestId("status-settings-auto-archive-unit"), {
        target: { value: "weeks" },
      });
      fireEvent.change(screen.getByTestId("status-settings-auto-archive-value"), {
        target: { value: "2" },
      });
      fireEvent.click(screen.getByTestId("status-settings-save"));

      const payload = mutateSpy.mock.calls[0][0] as {
        nextStatuses: StatusDefinition[];
      };
      expect(
        Number(payload.nextStatuses[0].auto_archive_after_seconds),
      ).toBe(1209600);
    });

    it("changing the unit preserves the displayed value and recomputes seconds", () => {
      const project = makeProject([
        makeStatus("in-progress", {
          // 12 days — not a whole number of weeks so inverse render lands
          // on "12 days", not "weeks".
          auto_archive_after_seconds: 1036800 as unknown as bigint,
        }),
      ]);
      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="in-progress"
          issueCount={0}
        />,
      );

      const value = screen.getByTestId(
        "status-settings-auto-archive-value",
      ) as HTMLInputElement;
      expect(value.value).toBe("12");

      // Switching the unit keeps the display value (12) and re-bases the
      // seconds against the new unit (12 weeks = 7257600s), rather than
      // recomputing the displayed number off the persisted seconds.
      fireEvent.change(screen.getByTestId("status-settings-auto-archive-unit"), {
        target: { value: "weeks" },
      });
      expect(value.value).toBe("12");

      fireEvent.click(screen.getByTestId("status-settings-save"));
      const payload = mutateSpy.mock.calls[0][0] as {
        nextStatuses: StatusDefinition[];
      };
      expect(
        Number(payload.nextStatuses[0].auto_archive_after_seconds),
      ).toBe(12 * 7 * 86400);
    });

    it("Clearing the value persists auto_archive_after_seconds: null", () => {
      const project = makeProject([
        makeStatus("in-progress", {
          auto_archive_after_seconds: 3600 as unknown as bigint,
        }),
      ]);
      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="in-progress"
          issueCount={0}
        />,
      );

      fireEvent.change(screen.getByTestId("status-settings-auto-archive-value"), {
        target: { value: "" },
      });
      fireEvent.click(screen.getByTestId("status-settings-save"));

      const payload = mutateSpy.mock.calls[0][0] as {
        nextStatuses: StatusDefinition[];
      };
      expect(
        payload.nextStatuses[0].auto_archive_after_seconds ?? null,
      ).toBeNull();
    });
  });

  describe("assignee picker", () => {
    it("renders only the Unassigned row when no agents/users are loaded", () => {
      const project = makeProject([
        makeStatus("open"),
        makeStatus("in-progress"),
      ]);
      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="open"
          issueCount={0}
        />,
      );
      // useAgents/useUsers are mocked to return [] above, so opening the
      // picker should surface only the Unassigned row — no agent/user
      // sections.
      fireEvent.click(screen.getByLabelText("Assign to"));
      const menu = screen.getByRole("menu");
      const rows = within(menu).getAllByRole("menuitem");
      expect(rows).toHaveLength(1);
      expect(rows[0].textContent).toContain("Unassigned");
    });

    it("shows the existing agent assignment in the trigger pill", () => {
      const project = makeProject([
        makeStatus("open", {
          on_enter: {
            assign_to: { Agent: { name: "swe" } },
            attach_form: null,
          },
        }),
      ]);
      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="open"
          issueCount={0}
        />,
      );
      // The pill renders the avatar (mocked) + name, so the trigger button's
      // aria-label is the picker caption and its text content is the name.
      expect(screen.getByLabelText("Assign to").textContent).toContain("swe");
    });

    it("picking Unassigned clears on_enter.assign_to", () => {
      const project = makeProject([
        makeStatus("open", {
          on_enter: {
            assign_to: { User: { name: "alice" } },
            attach_form: null,
          },
        }),
      ]);
      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="open"
          issueCount={0}
        />,
      );
      fireEvent.click(screen.getByLabelText("Assign to"));
      const menu = screen.getByRole("menu");
      const rows = within(menu).getAllByRole("menuitem");
      fireEvent.click(rows[0]);
      fireEvent.click(screen.getByTestId("status-settings-save"));
      const payload = mutateSpy.mock.calls[0][0] as {
        nextStatuses: StatusDefinition[];
      };
      expect(payload.nextStatuses[0].on_enter).toBeNull();
    });
  });

  describe("max simultaneous sessions", () => {
    it("renders empty when the status has no cap", () => {
      const project = makeProject([makeStatus("in-progress")]);
      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="in-progress"
          issueCount={0}
        />,
      );
      const input = screen.getByTestId(
        "status-settings-max-simultaneous-sessions",
      ) as HTMLInputElement;
      expect(input.value).toBe("");
    });

    it("renders the existing cap when set", () => {
      const project = makeProject([
        makeStatus("in-progress", {
          max_simultaneous_sessions: 4,
        }),
      ]);
      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="in-progress"
          issueCount={0}
        />,
      );
      const input = screen.getByTestId(
        "status-settings-max-simultaneous-sessions",
      ) as HTMLInputElement;
      expect(input.value).toBe("4");
    });

    it("Save persists the new cap as a number", () => {
      const project = makeProject([makeStatus("in-progress")]);
      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="in-progress"
          issueCount={0}
        />,
      );
      fireEvent.change(
        screen.getByTestId("status-settings-max-simultaneous-sessions"),
        { target: { value: "7" } },
      );
      fireEvent.click(screen.getByTestId("status-settings-save"));
      const payload = mutateSpy.mock.calls[0][0] as {
        nextStatuses: StatusDefinition[];
      };
      expect(payload.nextStatuses[0].max_simultaneous_sessions).toBe(7);
    });

    it("Clearing the input persists max_simultaneous_sessions: null", () => {
      const project = makeProject([
        makeStatus("in-progress", { max_simultaneous_sessions: 3 }),
      ]);
      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="in-progress"
          issueCount={0}
        />,
      );
      fireEvent.change(
        screen.getByTestId("status-settings-max-simultaneous-sessions"),
        { target: { value: "" } },
      );
      fireEvent.click(screen.getByTestId("status-settings-save"));
      const payload = mutateSpy.mock.calls[0][0] as {
        nextStatuses: StatusDefinition[];
      };
      expect(
        payload.nextStatuses[0].max_simultaneous_sessions ?? null,
      ).toBeNull();
    });
  });

  describe("session_settings", () => {
    it("renders all session_settings inputs as empty when the status has no overrides", () => {
      const project = makeProject([makeStatus("in-progress")]);
      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="in-progress"
          issueCount={0}
        />,
      );
      expect(
        (screen.getByTestId("status-settings-cpu-limit") as HTMLInputElement).value,
      ).toBe("");
      expect(
        (screen.getByTestId("status-settings-memory-limit") as HTMLInputElement).value,
      ).toBe("");
      expect(
        (screen.getByTestId("status-settings-image") as HTMLInputElement).value,
      ).toBe("");
      expect(
        (screen.getByTestId("status-settings-model") as HTMLInputElement).value,
      ).toBe("");
      expect(
        (screen.getByTestId("status-settings-branch") as HTMLInputElement).value,
      ).toBe("");
      expect(
        (screen.getByTestId("status-settings-max-retries") as HTMLInputElement).value,
      ).toBe("");
      // Idle timeout uses the design-system Picker — verify the pill content
      // rather than reaching for a hidden `value` attribute.
      expect(
        screen.getByLabelText("Idle timeout").textContent,
      ).toContain("Server default");
      // The inline seconds input is only rendered in the "seconds" mode.
      expect(
        screen.queryByTestId("status-settings-idle-timeout-seconds"),
      ).toBeNull();
    });

    it("seeds inputs from an existing session_settings override", () => {
      const project = makeProject([
        makeStatus("in-progress", {
          session_settings: {
            cpu_limit: "500m",
            memory_limit: "1Gi",
            image: "ghcr.io/org/img:v1",
            model: "claude-opus-4-7",
            branch: "main",
            max_retries: 3,
            idle_timeout: { kind: "seconds", value: 600 as unknown as bigint },
          },
        }),
      ]);
      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="in-progress"
          issueCount={0}
        />,
      );
      expect(
        (screen.getByTestId("status-settings-cpu-limit") as HTMLInputElement).value,
      ).toBe("500m");
      expect(
        (screen.getByTestId("status-settings-memory-limit") as HTMLInputElement).value,
      ).toBe("1Gi");
      expect(
        (screen.getByTestId("status-settings-image") as HTMLInputElement).value,
      ).toBe("ghcr.io/org/img:v1");
      expect(
        (screen.getByTestId("status-settings-model") as HTMLInputElement).value,
      ).toBe("claude-opus-4-7");
      expect(
        (screen.getByTestId("status-settings-branch") as HTMLInputElement).value,
      ).toBe("main");
      expect(
        (screen.getByTestId("status-settings-max-retries") as HTMLInputElement).value,
      ).toBe("3");
      expect(
        screen.getByLabelText("Idle timeout").textContent,
      ).toContain("600 seconds");
      expect(
        (screen.getByTestId(
          "status-settings-idle-timeout-seconds",
        ) as HTMLInputElement).value,
      ).toBe("600");
    });

    it("Save persists the edited session_settings fields", () => {
      const project = makeProject([makeStatus("in-progress")]);
      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="in-progress"
          issueCount={0}
        />,
      );
      fireEvent.change(screen.getByTestId("status-settings-cpu-limit"), {
        target: { value: "750m" },
      });
      fireEvent.change(screen.getByTestId("status-settings-memory-limit"), {
        target: { value: "2Gi" },
      });
      fireEvent.change(screen.getByTestId("status-settings-model"), {
        target: { value: "claude-sonnet-4-6" },
      });
      fireEvent.change(screen.getByTestId("status-settings-max-retries"), {
        target: { value: "5" },
      });
      fireEvent.click(screen.getByTestId("status-settings-save"));

      const payload = mutateSpy.mock.calls[0][0] as {
        nextStatuses: StatusDefinition[];
      };
      const ss = payload.nextStatuses[0].session_settings;
      expect(ss?.cpu_limit).toBe("750m");
      expect(ss?.memory_limit).toBe("2Gi");
      expect(ss?.model).toBe("claude-sonnet-4-6");
      expect(ss?.max_retries).toBe(5);
    });

    it("clearing every session_settings field collapses session_settings back to undefined", () => {
      const project = makeProject([
        makeStatus("in-progress", {
          session_settings: {
            cpu_limit: "500m",
          },
        }),
      ]);
      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="in-progress"
          issueCount={0}
        />,
      );
      fireEvent.change(screen.getByTestId("status-settings-cpu-limit"), {
        target: { value: "" },
      });
      fireEvent.click(screen.getByTestId("status-settings-save"));

      const payload = mutateSpy.mock.calls[0][0] as {
        nextStatuses: StatusDefinition[];
      };
      expect(payload.nextStatuses[0].session_settings).toBeUndefined();
    });

    describe("idle timeout", () => {
      // Helper: click the Idle timeout picker open, then click the row whose
      // visible text matches `rowLabel`. Mirrors the assignee-picker test
      // pattern — pickers are disambiguated by their `aria-label`.
      function pickIdleTimeoutRow(rowLabel: string) {
        fireEvent.click(screen.getByLabelText("Idle timeout"));
        const menu = screen.getByRole("menu");
        const rows = within(menu).getAllByRole("menuitem");
        const row = rows.find((r) => r.textContent?.includes(rowLabel));
        if (!row) throw new Error(`No row matching '${rowLabel}'`);
        fireEvent.click(row);
      }

      it("switching to Never persists a Timeout::Infinite", () => {
        const project = makeProject([makeStatus("in-progress")]);
        render(
          <StatusSettingsModal
            open={true}
            onClose={() => {}}
            projectRecord={project}
            statusKey="in-progress"
            issueCount={0}
          />,
        );
        pickIdleTimeoutRow("Never");
        fireEvent.click(screen.getByTestId("status-settings-save"));
        const payload = mutateSpy.mock.calls[0][0] as {
          nextStatuses: StatusDefinition[];
        };
        expect(
          payload.nextStatuses[0].session_settings?.idle_timeout,
        ).toEqual({ kind: "infinite" });
      });

      it("switching to Custom and entering a value persists Timeout::Seconds", () => {
        const project = makeProject([makeStatus("in-progress")]);
        render(
          <StatusSettingsModal
            open={true}
            onClose={() => {}}
            projectRecord={project}
            statusKey="in-progress"
            issueCount={0}
          />,
        );
        pickIdleTimeoutRow("Custom");
        fireEvent.change(
          screen.getByTestId("status-settings-idle-timeout-seconds"),
          { target: { value: "300" } },
        );
        fireEvent.click(screen.getByTestId("status-settings-save"));
        const payload = mutateSpy.mock.calls[0][0] as {
          nextStatuses: StatusDefinition[];
        };
        const t = payload.nextStatuses[0].session_settings?.idle_timeout;
        expect(t?.kind).toBe("seconds");
        if (t?.kind === "seconds") {
          expect(Number(t.value)).toBe(300);
        }
      });

      it("switching back to Server default clears idle_timeout (and collapses session_settings)", () => {
        const project = makeProject([
          makeStatus("in-progress", {
            session_settings: {
              idle_timeout: {
                kind: "seconds",
                value: 600 as unknown as bigint,
              },
            },
          }),
        ]);
        render(
          <StatusSettingsModal
            open={true}
            onClose={() => {}}
            projectRecord={project}
            statusKey="in-progress"
            issueCount={0}
          />,
        );
        pickIdleTimeoutRow("Server default");
        fireEvent.click(screen.getByTestId("status-settings-save"));
        const payload = mutateSpy.mock.calls[0][0] as {
          nextStatuses: StatusDefinition[];
        };
        // Only idle_timeout was set, so the whole session_settings collapses
        // back to undefined to keep the wire body slim.
        expect(payload.nextStatuses[0].session_settings).toBeUndefined();
      });

      it("inline seconds input only renders in Custom mode", () => {
        const project = makeProject([makeStatus("in-progress")]);
        render(
          <StatusSettingsModal
            open={true}
            onClose={() => {}}
            projectRecord={project}
            statusKey="in-progress"
            issueCount={0}
          />,
        );
        // Default mode: no seconds input.
        expect(
          screen.queryByTestId("status-settings-idle-timeout-seconds"),
        ).toBeNull();
        pickIdleTimeoutRow("Custom");
        expect(
          screen.getByTestId("status-settings-idle-timeout-seconds"),
        ).toBeDefined();
        pickIdleTimeoutRow("Never");
        expect(
          screen.queryByTestId("status-settings-idle-timeout-seconds"),
        ).toBeNull();
      });
    });
  });

  describe("suppress_sessions", () => {
    it("renders unchecked when the status has no suppress_sessions flag", () => {
      const project = makeProject([makeStatus("in-progress")]);
      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="in-progress"
          issueCount={0}
        />,
      );
      const toggle = screen.getByTestId(
        "status-settings-suppress-sessions",
      ) as HTMLInputElement;
      expect(toggle.checked).toBe(false);
    });

    it("Save persists suppress_sessions=true when checked", () => {
      const project = makeProject([makeStatus("in-progress")]);
      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="in-progress"
          issueCount={0}
        />,
      );
      fireEvent.click(
        screen.getByTestId("status-settings-suppress-sessions"),
      );
      fireEvent.click(screen.getByTestId("status-settings-save"));
      const payload = mutateSpy.mock.calls[0][0] as {
        nextStatuses: StatusDefinition[];
      };
      expect(payload.nextStatuses[0].suppress_sessions).toBe(true);
    });

    it("renders checked when the status carries suppress_sessions=true", () => {
      const project = makeProject([
        makeStatus("in-progress", { suppress_sessions: true }),
      ]);
      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="in-progress"
          issueCount={0}
        />,
      );
      const toggle = screen.getByTestId(
        "status-settings-suppress-sessions",
      ) as HTMLInputElement;
      expect(toggle.checked).toBe(true);
    });
  });

  describe("on-enter flags (clear_assignee, teardown_work)", () => {
    it("checking teardown_work materializes on_enter from null with just that flag set", () => {
      const project = makeProject([makeStatus("in-progress")]);
      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="in-progress"
          issueCount={0}
        />,
      );
      fireEvent.click(screen.getByTestId("status-settings-teardown-work"));
      fireEvent.click(screen.getByTestId("status-settings-save"));
      const payload = mutateSpy.mock.calls[0][0] as {
        nextStatuses: StatusDefinition[];
      };
      const onEnter = payload.nextStatuses[0].on_enter;
      expect(onEnter).not.toBeNull();
      expect(onEnter?.teardown_work).toBe(true);
      expect(onEnter?.clear_assignee).toBe(false);
      expect(onEnter?.assign_to ?? null).toBeNull();
    });

    it("checking clear_assignee persists clear_assignee=true", () => {
      const project = makeProject([makeStatus("in-progress")]);
      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="in-progress"
          issueCount={0}
        />,
      );
      fireEvent.click(screen.getByTestId("status-settings-clear-assignee"));
      fireEvent.click(screen.getByTestId("status-settings-save"));
      const payload = mutateSpy.mock.calls[0][0] as {
        nextStatuses: StatusDefinition[];
      };
      expect(payload.nextStatuses[0].on_enter?.clear_assignee).toBe(true);
    });

    it("renders checked from the existing on_enter flags", () => {
      const project = makeProject([
        makeStatus("closed", {
          on_enter: {
            assign_to: null,
            attach_form: null,
            clear_assignee: true,
            teardown_work: true,
          },
        }),
      ]);
      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="closed"
          issueCount={0}
        />,
      );
      expect(
        (screen.getByTestId(
          "status-settings-clear-assignee",
        ) as HTMLInputElement).checked,
      ).toBe(true);
      expect(
        (screen.getByTestId(
          "status-settings-teardown-work",
        ) as HTMLInputElement).checked,
      ).toBe(true);
    });

    it("unchecking the last on_enter flag collapses on_enter back to null", () => {
      const project = makeProject([
        makeStatus("closed", {
          on_enter: {
            assign_to: null,
            attach_form: null,
            clear_assignee: false,
            teardown_work: true,
          },
        }),
      ]);
      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="closed"
          issueCount={0}
        />,
      );
      fireEvent.click(screen.getByTestId("status-settings-teardown-work"));
      fireEvent.click(screen.getByTestId("status-settings-save"));
      const payload = mutateSpy.mock.calls[0][0] as {
        nextStatuses: StatusDefinition[];
      };
      expect(payload.nextStatuses[0].on_enter).toBeNull();
    });

    it("checking clear_assignee clears a previously set assignee", () => {
      const project = makeProject([
        makeStatus("closed", {
          on_enter: {
            assign_to: { Agent: { name: "swe" } },
            attach_form: null,
          },
        }),
      ]);
      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="closed"
          issueCount={0}
        />,
      );
      fireEvent.click(screen.getByTestId("status-settings-clear-assignee"));
      fireEvent.click(screen.getByTestId("status-settings-save"));
      const payload = mutateSpy.mock.calls[0][0] as {
        nextStatuses: StatusDefinition[];
      };
      const onEnter = payload.nextStatuses[0].on_enter;
      expect(onEnter?.clear_assignee).toBe(true);
      expect(onEnter?.assign_to ?? null).toBeNull();
    });

    it("picking an assignee clears a previously set clear_assignee", () => {
      agentsList = [{ name: "swe" }];
      const project = makeProject([
        makeStatus("closed", {
          on_enter: {
            assign_to: null,
            attach_form: null,
            clear_assignee: true,
            teardown_work: false,
          },
        }),
      ]);
      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="closed"
          issueCount={0}
        />,
      );
      // Sanity: the on_enter starts with clear_assignee set.
      expect(
        (screen.getByTestId(
          "status-settings-clear-assignee",
        ) as HTMLInputElement).checked,
      ).toBe(true);

      fireEvent.click(screen.getByLabelText("Assign to"));
      const menu = screen.getByRole("menu");
      const rows = within(menu).getAllByRole("menuitem");
      // Rows: [Unassigned, swe]. Pick the agent row.
      fireEvent.click(rows[1]);
      fireEvent.click(screen.getByTestId("status-settings-save"));
      const payload = mutateSpy.mock.calls[0][0] as {
        nextStatuses: StatusDefinition[];
      };
      const onEnter = payload.nextStatuses[0].on_enter;
      expect(onEnter?.clear_assignee).toBe(false);
      expect(onEnter?.assign_to).toEqual({ Agent: { name: "swe" } });
    });
  });
});
