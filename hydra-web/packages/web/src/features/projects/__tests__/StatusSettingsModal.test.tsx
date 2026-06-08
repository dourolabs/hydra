// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { ProjectRecord, StatusDefinition } from "@hydra/api";
import type { ReactNode } from "react";

const mutateSpy = vi.fn();
const addToastSpy = vi.fn();
let mutationPending = false;

vi.mock("@tanstack/react-query", () => ({
  useMutation: ({
    onSuccess,
  }: {
    onSuccess?: (response: { project_id: string }) => void;
  }) => ({
    mutate: (vars: unknown) => {
      mutateSpy(vars);
      // Synchronously fire success so onClose / invalidate run in tests.
      onSuccess?.({ project_id: "j-eng" });
    },
    isPending: mutationPending,
  }),
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
    mutationPending = false;
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
});
