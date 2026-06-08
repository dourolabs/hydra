// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
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
    disabled,
    "data-testid": testId,
  }: {
    label?: string;
    value: string;
    onChange?: (e: { target: { value: string } }) => void;
    placeholder?: string;
    required?: boolean;
    disabled?: boolean;
    "data-testid"?: string;
  }) => (
    <label>
      {label}
      <input
        value={value}
        disabled={disabled}
        onChange={(e) =>
          onChange?.({ target: { value: e.target.value } })
        }
        placeholder={placeholder}
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
        onChange={(e) =>
          onChange({ target: { value: e.target.value } })
        }
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
}));

const updateProjectSpy = vi.fn(async (_id: string, req: unknown) => req);
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
      default_status_key: statuses[0]?.key ?? ("open" as never),
      creator: "alice" as never,
      deleted: false,
      prompt_path: null,
    },
  };
}

describe("StatusSettingsModal", () => {
  beforeEach(() => {
    mutateSpy.mockReset();
    addToastSpy.mockReset();
    updateProjectSpy.mockClear();
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
    const key = screen.getByTestId("status-settings-key") as HTMLInputElement;
    expect(key.value).toBe("in-progress");
    expect(key.disabled).toBe(true);
  });

  it("Save fires updateProject with the modified status in the full project shape", () => {
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
    const next = mutateSpy.mock.calls[0][0] as StatusDefinition[];
    expect(next).toHaveLength(2);
    expect(next[0].key).toBe("open");
    expect(next[1].key).toBe("in-progress");
    expect(next[1].label).toBe("Doing");
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
    const next = mutateSpy.mock.calls[0][0] as StatusDefinition[];
    expect(next.map((s) => s.key)).toEqual(["open", "closed"]);
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
    it("renders only a Name input and inline Prompt editor — no Key, no advanced fields", () => {
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

      const name = screen.getByTestId("status-settings-name") as HTMLInputElement;
      expect(name.value).toBe("");
      // Inline markdown editor is mounted directly (no toggle, no path field).
      expect(screen.getByTestId("status-settings-prompt-body")).toBeDefined();
      // Advanced edit-mode UI must not appear in the simplified Add flow.
      expect(screen.queryByTestId("status-settings-key")).toBeNull();
      expect(screen.queryByTestId("status-settings-label")).toBeNull();
      expect(screen.queryByTestId("status-settings-prompt-path")).toBeNull();
      expect(screen.queryByTestId("status-settings-prompt-path-toggle")).toBeNull();
      expect(screen.queryByTestId("status-settings-assign-kind")).toBeNull();
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

    it("Save derives the key by slugifying the name and appends the status with an auto prompt_path", async () => {
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

      fireEvent.change(screen.getByTestId("status-settings-name"), {
        target: { value: "In Review" },
      });
      await act(async () => {
        fireEvent.click(screen.getByTestId("status-settings-save"));
      });

      expect(updateProjectSpy).toHaveBeenCalledTimes(1);
      const projectPayload = updateProjectSpy.mock.calls[0][1] as {
        project: { statuses: StatusDefinition[] };
      };
      expect(projectPayload.project.statuses.map((s) => s.key)).toEqual([
        "open",
        "in-progress",
        "in-review",
      ]);
      const added = projectPayload.project.statuses[2];
      expect(added.label).toBe("In Review");
      expect(added.prompt_path).toBe("/projects/engineering/statuses/in-review.md");
    });

    it("disables Save and shows error when the derived key collides", () => {
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

      fireEvent.change(screen.getByTestId("status-settings-name"), {
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

      fireEvent.change(screen.getByTestId("status-settings-name"), {
        target: { value: "@@@" },
      });
      const save = screen.getByTestId("status-settings-save") as HTMLButtonElement;
      expect(save.disabled).toBe(true);
      expect(
        screen.getByTestId("status-settings-new-error").textContent,
      ).toContain("letter or digit");
    });

    it("writes the prompt document at the auto path before creating the status", async () => {
      const project = makeProject([makeStatus("open")]);
      // No existing doc at the auto path → 404 → createDocument fires.
      getDocumentByPathSpy.mockImplementation(async () => {
        throw new ApiErrorMock(404, "not found");
      });
      createDocumentSpy.mockImplementation(async () => ({
        document_id: "d-new",
      }));

      render(
        <StatusSettingsModal
          open={true}
          mode="new"
          onClose={() => {}}
          projectRecord={project}
        />,
      );
      fireEvent.change(screen.getByTestId("status-settings-name"), {
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
      expect(updateProjectSpy).toHaveBeenCalledTimes(1);
    });

    it("upserts the prompt document when one already exists at the auto path", async () => {
      const project = makeProject([makeStatus("open")]);
      getDocumentByPathSpy.mockImplementation(async () => ({
        document_id: "d-existing",
        version: 4,
        timestamp: "2026-01-01T00:00:00Z",
        document: {
          title: "old title",
          body_markdown: "old body",
          path: "/projects/engineering/statuses/blocked.md",
        },
      }));
      updateDocumentSpy.mockImplementation(async () => ({
        document_id: "d-existing",
      }));

      render(
        <StatusSettingsModal
          open={true}
          mode="new"
          onClose={() => {}}
          projectRecord={project}
        />,
      );
      fireEvent.change(screen.getByTestId("status-settings-name"), {
        target: { value: "Blocked" },
      });
      fireEvent.change(screen.getByTestId("status-settings-prompt-body"), {
        target: { value: "Fresh body" },
      });
      await act(async () => {
        fireEvent.click(screen.getByTestId("status-settings-save"));
      });

      expect(createDocumentSpy).not.toHaveBeenCalled();
      expect(updateDocumentSpy).toHaveBeenCalledTimes(1);
      const docCall = updateDocumentSpy.mock.calls[0];
      expect(docCall[0]).toBe("d-existing");
      expect(
        (docCall[1] as { document: { body_markdown: string } }).document
          .body_markdown,
      ).toBe("Fresh body");
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
      fireEvent.change(screen.getByTestId("status-settings-name"), {
        target: { value: "Blocked" },
      });
      await act(async () => {
        fireEvent.click(screen.getByTestId("status-settings-save"));
      });

      expect(getDocumentByPathSpy).not.toHaveBeenCalled();
      expect(createDocumentSpy).not.toHaveBeenCalled();
      expect(updateDocumentSpy).not.toHaveBeenCalled();
      // The status still records the auto path so a later editor knows
      // where to write the document.
      const projectPayload = updateProjectSpy.mock.calls[0][1] as {
        project: { statuses: StatusDefinition[] };
      };
      expect(projectPayload.project.statuses[1].prompt_path).toBe(
        "/projects/engineering/statuses/blocked.md",
      );
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

      fireEvent.change(screen.getByTestId("status-settings-name"), {
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

      // Project save fires after all issues moved.
      expect(updateProjectSpy).toHaveBeenCalledTimes(1);
      const projectPayload = updateProjectSpy.mock.calls[0][1] as {
        project: { statuses: StatusDefinition[]; default_status_key: string };
      };
      expect(projectPayload.project.statuses.map((s) => s.key)).toEqual([
        "open",
        "closed",
      ]);
      expect(projectPayload.project.default_status_key).toBe("open");
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

    it("retargets default_status_key to the left neighbor when deleting the default", async () => {
      const project = makeProject([
        makeStatus("open"),
        makeStatus("in-progress"),
        makeStatus("closed"),
      ]);
      // Make "in-progress" the project's default so deleting it forces a
      // reassign.
      project.project.default_status_key = "in-progress" as never;
      listIssuesSpy.mockImplementationOnce(async () => ({
        issues: [{ issue_id: "i-aaa" }],
        next_cursor: null,
      }));

      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="in-progress"
          issueCount={1}
        />,
      );

      fireEvent.click(screen.getByTestId("status-settings-delete"));
      await act(async () => {
        fireEvent.click(screen.getByTestId("status-settings-move-confirm"));
      });

      const payload = updateProjectSpy.mock.calls[0][1] as {
        project: { default_status_key: string };
      };
      // Left neighbor of "in-progress" is "open".
      expect(payload.project.default_status_key).toBe("open");
    });

    it("retargets default_status_key to the right neighbor when deleting the leftmost default", async () => {
      const project = makeProject([
        makeStatus("open"),
        makeStatus("in-progress"),
        makeStatus("closed"),
      ]);
      // Default is "open" (leftmost) and we delete it — fall back to the
      // right neighbor.
      listIssuesSpy.mockImplementationOnce(async () => ({
        issues: [{ issue_id: "i-aaa" }],
        next_cursor: null,
      }));

      render(
        <StatusSettingsModal
          open={true}
          onClose={() => {}}
          projectRecord={project}
          statusKey="open"
          issueCount={1}
        />,
      );

      fireEvent.click(screen.getByTestId("status-settings-delete"));
      // The default neighbor target for "open" is "in-progress", so we can
      // confirm without changing the select.
      await act(async () => {
        fireEvent.click(screen.getByTestId("status-settings-move-confirm"));
      });

      const payload = updateProjectSpy.mock.calls[0][1] as {
        project: { default_status_key: string };
      };
      expect(payload.project.default_status_key).toBe("in-progress");
    });
  });

  describe("Prompt document editor (PR-11)", () => {
    it("renders the PromptDocumentEditor in place of the prompt path Input", () => {
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
      // Container + toggle are owned by PromptDocumentEditor.
      expect(
        screen.getByTestId("status-settings-prompt-path-container"),
      ).toBeDefined();
      expect(
        screen.getByTestId("status-settings-prompt-path-toggle"),
      ).toBeDefined();
      // Path input still exposes the same testId for editing.
      expect(screen.getByTestId("status-settings-prompt-path")).toBeDefined();
      // Collapsed by default — textarea/save are not mounted.
      expect(
        screen.queryByTestId("status-settings-prompt-path-textarea"),
      ).toBeNull();
      expect(
        screen.queryByTestId("status-settings-prompt-path-save"),
      ).toBeNull();
      // Critically: no doc fetch is triggered while collapsed.
      expect(lastPromptQueryFnRef.fn).toBeNull();
    });

    it("expanding fetches the doc at /projects/<key>/statuses/<status-key>.md by default", () => {
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

      fireEvent.click(
        screen.getByTestId("status-settings-prompt-path-toggle"),
      );
      // The body mounts and registers a useQuery against the default path
      // since the status has no prompt_path set.
      expect(lastPromptQueryKeyRef.key).toEqual([
        "documentByPath",
        "/projects/engineering/statuses/in-progress.md",
      ]);
    });

    it("uses the existing prompt_path over the default when one is set", () => {
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

      fireEvent.click(
        screen.getByTestId("status-settings-prompt-path-toggle"),
      );
      expect(lastPromptQueryKeyRef.key).toEqual([
        "documentByPath",
        "/custom/status.md",
      ]);
    });

    it("editing the path input persists into the modal draft via patch", () => {
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
      fireEvent.change(screen.getByTestId("status-settings-prompt-path"), {
        target: { value: "/my/explicit/path.md" },
      });
      fireEvent.click(screen.getByTestId("status-settings-save"));
      const next = mutateSpy.mock.calls[0][0] as StatusDefinition[];
      expect(next[1].prompt_path).toBe("/my/explicit/path.md");
    });
  });

  it("setKind('user') is a no-op when no users are loaded", () => {
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
    // useUsers is mocked to return [] above, so flipping to "user" should
    // not produce an on_enter assignment with an empty Principal name.
    fireEvent.change(screen.getByTestId("status-settings-assign-kind"), {
      target: { value: "user" },
    });
    fireEvent.click(screen.getByTestId("status-settings-save"));
    const next = mutateSpy.mock.calls[0][0] as StatusDefinition[];
    expect(next[0].on_enter).toBeNull();
  });
});
