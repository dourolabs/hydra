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

vi.mock("@tanstack/react-query", () => ({
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
      // before either firing the success path or invoking onError.
      void (async () => {
        const ctx = await Promise.resolve(onMutate?.(vars));
        if (simulateError) {
          onError?.(simulateError, vars, ctx);
          perCall?.onError?.(simulateError, vars, ctx);
          return;
        }
        void mutationFn?.(vars);
        const response = { project_id: "j-eng" };
        onSuccess?.(response, vars);
        perCall?.onSuccess?.(response, vars);
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
}));

const updateProjectSpy = vi.fn(async (_id: string, req: unknown) => req);
vi.mock("../../../api/client", () => ({
  apiClient: {
    updateProject: updateProjectSpy,
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
    cancelQueriesSpy.mockClear();
    setQueryDataSpy.mockClear();
    invalidateQueriesSpy.mockClear();
    queryDataByKey = new Map();
    mutationPending = false;
    simulateError = null;
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

  it("Move right swaps the status with its neighbor and persists", () => {
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
        issueCount={0}
      />,
    );

    fireEvent.click(screen.getByTestId("status-settings-move-right"));

    const next = mutateSpy.mock.calls[0][0] as StatusDefinition[];
    expect(next.map((s) => s.key)).toEqual(["in-progress", "open", "closed"]);
  });

  it("Move left is disabled at the first position", () => {
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
    const btn = screen.getByTestId("status-settings-move-left") as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
  });

  it("Delete is disabled with tooltip when issueCount > 0", () => {
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
        issueCount={3}
      />,
    );

    const del = screen.getByTestId("status-settings-delete") as HTMLButtonElement;
    expect(del.disabled).toBe(true);
    expect(del.title).toBe(
      "Cannot delete a status with 3 open issues; move them first",
    );
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

  it("Move does NOT close the modal (user can keep nudging the column)", () => {
    const project = makeProject([
      makeStatus("open"),
      makeStatus("in-progress"),
      makeStatus("closed"),
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

    fireEvent.click(screen.getByTestId("status-settings-move-right"));
    expect(mutateSpy).toHaveBeenCalledTimes(1);
    expect(onClose).not.toHaveBeenCalled();
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
    it("opens with a blank draft and an editable Key field", () => {
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

      const key = screen.getByTestId("status-settings-key") as HTMLInputElement;
      expect(key.disabled).toBe(false);
      // blankStatus(2) → key === "status-3" because there are already 2 statuses.
      expect(key.value).toBe("status-3");

      const label = screen.getByTestId("status-settings-label") as HTMLInputElement;
      expect(label.value).toBe("");
    });

    it("default color cycles from LABEL_COLOR_PALETTE by statuses.length", () => {
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
      // ColorPicker is mocked to render `data-testid="color-<value>"`.
      // Mocked palette = ["#111", "#222", "#333", "#444", "#555", "#666"].
      // With 2 existing statuses, blankStatus(2) picks index 2 → "#333".
      expect(screen.getByTestId("color-#333")).toBeDefined();
    });

    it("hides the move and delete controls in new mode", () => {
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

      expect(screen.queryByTestId("status-settings-move-left")).toBeNull();
      expect(screen.queryByTestId("status-settings-move-right")).toBeNull();
      expect(screen.queryByTestId("status-settings-delete")).toBeNull();
    });

    it("Save appends the new status to the project's statuses array", () => {
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

      fireEvent.change(screen.getByTestId("status-settings-key"), {
        target: { value: "blocked" },
      });
      fireEvent.change(screen.getByTestId("status-settings-label"), {
        target: { value: "Blocked" },
      });
      fireEvent.click(screen.getByTestId("status-settings-save"));

      expect(mutateSpy).toHaveBeenCalledTimes(1);
      const next = mutateSpy.mock.calls[0][0] as StatusDefinition[];
      expect(next.map((s) => s.key)).toEqual(["open", "in-progress", "blocked"]);
      expect(next[2].label).toBe("Blocked");
    });

    it("disables Save and shows error when key collides with an existing status", () => {
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

      fireEvent.change(screen.getByTestId("status-settings-key"), {
        target: { value: "open" },
      });
      fireEvent.change(screen.getByTestId("status-settings-label"), {
        target: { value: "Open" },
      });

      const save = screen.getByTestId("status-settings-save") as HTMLButtonElement;
      expect(save.disabled).toBe(true);
      expect(screen.getByTestId("status-settings-new-error").textContent).toContain(
        "already exists",
      );
    });

    it("rejects invalid key characters", () => {
      const project = makeProject([makeStatus("open")]);
      render(
        <StatusSettingsModal
          open={true}
          mode="new"
          onClose={() => {}}
          projectRecord={project}
        />,
      );

      fireEvent.change(screen.getByTestId("status-settings-key"), {
        target: { value: "Has Spaces" },
      });
      const save = screen.getByTestId("status-settings-save") as HTMLButtonElement;
      expect(save.disabled).toBe(true);
      expect(screen.getByTestId("status-settings-new-error").textContent).toContain(
        "lowercase letters",
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

      fireEvent.change(screen.getByTestId("status-settings-key"), {
        target: { value: "blocked" },
      });
      fireEvent.change(screen.getByTestId("status-settings-label"), {
        target: { value: "Blocked" },
      });
      await act(async () => {
        fireEvent.click(screen.getByTestId("status-settings-save"));
      });
      expect(onClose).toHaveBeenCalledTimes(1);
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
