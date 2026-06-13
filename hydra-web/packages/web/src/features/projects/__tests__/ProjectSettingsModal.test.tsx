// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import type { ProjectRecord } from "@hydra/api";
import type { ReactNode } from "react";

const addToastSpy = vi.fn();
const navigateSpy = vi.fn();
let mutationPending = false;
const invalidateQueriesSpy = vi.fn();
const cancelQueriesSpy = vi.fn(async () => {});
let projectsHookData: ProjectRecord[] | undefined = [];
let promptQueryState = {
  isLoading: false,
  isError: false,
  isSuccess: true,
  data: null as { document: { body_markdown: string } } | null,
  error: null as Error | null,
};

vi.mock("@tanstack/react-query", () => ({
  useQuery: () => promptQueryState,
  useMutation: ({
    mutationFn,
    onMutate,
    onSuccess,
    onError,
  }: {
    mutationFn?: () => Promise<{ project_id: string }>;
    onMutate?: () => Promise<unknown> | unknown;
    onSuccess?: (response: { project_id: string }) => void;
    onError?: (err: Error, vars: unknown, context: unknown) => void;
  }) => ({
    mutate: () => {
      void (async () => {
        const ctx = onMutate ? await Promise.resolve(onMutate()) : undefined;
        try {
          const response = (await mutationFn?.()) ?? { project_id: "j-engine" };
          onSuccess?.(response);
        } catch (err) {
          onError?.(err as Error, undefined, ctx);
        }
      })();
    },
    isPending: mutationPending,
  }),
  useQueryClient: () => ({
    cancelQueries: cancelQueriesSpy,
    getQueryData: () => undefined,
    setQueryData: () => {},
    invalidateQueries: invalidateQueriesSpy,
  }),
}));

vi.mock("react-router-dom", () => ({
  useNavigate: () => navigateSpy,
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
    onChange?: (e: { target: { value: string } }) => void;
    placeholder?: string;
    required?: boolean;
    "data-testid"?: string;
  }) => (
    <label>
      {label}
      <input
        value={value}
        onChange={(e) => onChange?.({ target: { value: e.target.value } })}
        placeholder={placeholder}
        data-testid={testId}
      />
    </label>
  ),
  Textarea: ({
    label,
    value,
    onChange,
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
        onChange={(e) => onChange?.({ target: { value: e.target.value } })}
        data-testid={testId}
      />
    </label>
  ),
  Modal: ({
    open,
    onClose,
    children,
    title,
  }: {
    open: boolean;
    onClose?: () => void;
    title?: string;
    children: ReactNode;
  }) =>
    open ? (
      <div role="dialog" aria-label={title} data-testid="modal">
        <div data-testid="modal-title">{title}</div>
        <button data-testid="modal-close" onClick={onClose}>
          Close
        </button>
        <div>{children}</div>
      </div>
    ) : null,
}));

const updateProjectSpy = vi.fn(async () => ({
  project_id: "j-engine",
  version: 2n,
}));
const archiveProjectSpy = vi.fn(async () => ({}));
const getDocumentByPathSpy = vi.fn();
const createDocumentSpy = vi.fn(async () => ({ document_id: "d-1" }));
const updateDocumentSpy = vi.fn(async () => ({ document_id: "d-1" }));
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
    archiveProject: archiveProjectSpy,
    getDocumentByPath: getDocumentByPathSpy,
    createDocument: createDocumentSpy,
    updateDocument: updateDocumentSpy,
  },
}));

vi.mock("../../toast/useToast", () => ({
  useToast: () => ({ addToast: addToastSpy }),
}));

vi.mock("../useProjects", () => ({
  useProjects: () => ({ data: projectsHookData }),
}));

vi.mock("../ProjectForm.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("../../../components/DeleteConfirmModal/DeleteConfirmModal", () => ({
  DeleteConfirmModal: ({
    open,
    onConfirm,
  }: {
    open: boolean;
    onConfirm: () => void;
  }) =>
    open ? (
      <button data-testid="confirm-delete" onClick={onConfirm}>
        Confirm delete
      </button>
    ) : null,
}));

const { ProjectSettingsModal } = await import("../ProjectSettingsModal");

function makeProject(): ProjectRecord {
  return {
    project_id: "j-engine" as never,
    version: 1,
    project: {
      key: "engineering" as never,
      name: "Engineering",
      statuses: [
        {
          key: "open" as never,
          label: "Open",
          color: "#3498db" as never,
          unblocks_parents: false,
          unblocks_dependents: false,
          cascades_to_children: false,
          on_enter: null,
          prompt_path: null,
          position: 0,
        },
      ],
      creator: "alice" as never,
      archived: false,
      prompt_path: "/projects/engineering/prompt.md" as never,
      priority: 7,
    },
  };
}

describe("ProjectSettingsModal", () => {
  beforeEach(() => {
    addToastSpy.mockReset();
    navigateSpy.mockReset();
    invalidateQueriesSpy.mockReset();
    updateProjectSpy.mockClear();
    archiveProjectSpy.mockClear();
    getDocumentByPathSpy.mockReset();
    createDocumentSpy.mockClear();
    updateDocumentSpy.mockClear();
    mutationPending = false;
    projectsHookData = [makeProject()];
    promptQueryState = {
      isLoading: false,
      isError: false,
      isSuccess: true,
      data: null,
      error: null,
    };
  });

  afterEach(() => {
    cleanup();
  });

  it("renders nothing when closed", () => {
    render(
      <ProjectSettingsModal
        open={false}
        onClose={() => {}}
        project={makeProject()}
      />,
    );
    expect(screen.queryByTestId("modal")).toBeNull();
  });

  it("renders the shared project form, pre-filled, with key/path notes and a delete control", () => {
    render(
      <ProjectSettingsModal open onClose={() => {}} project={makeProject()} />,
    );

    expect(screen.getByTestId("modal-title").textContent).toContain(
      "Engineering",
    );
    const name = screen.getByTestId("project-form-name") as HTMLInputElement;
    expect(name.value).toBe("Engineering");
    expect(screen.getByTestId("project-form-prompt-body")).toBeDefined();
    // Key + prompt path are shown read-only (not editable fields).
    expect(screen.getByTestId("project-form-key").textContent).toContain(
      "engineering",
    );
    expect(
      screen.getByTestId("project-form-prompt-path").textContent,
    ).toContain("/projects/engineering/prompt.md");
    // Edit mode exposes Delete project.
    expect(screen.getByTestId("project-form-delete")).toBeDefined();
  });

  it("re-derives the key and prompt path as the name changes", () => {
    render(
      <ProjectSettingsModal open onClose={() => {}} project={makeProject()} />,
    );
    fireEvent.change(screen.getByTestId("project-form-name"), {
      target: { value: "Platform Eng" },
    });
    expect(screen.getByTestId("project-form-key").textContent).toContain(
      "platform-eng",
    );
    expect(
      screen.getByTestId("project-form-prompt-path").textContent,
    ).toContain("/projects/platform-eng/prompt.md");
  });

  it("Save updates the project with the re-derived key, preserving existing statuses", async () => {
    const onClose = vi.fn();
    render(
      <ProjectSettingsModal open onClose={onClose} project={makeProject()} />,
    );

    fireEvent.change(screen.getByTestId("project-form-name"), {
      target: { value: "Platform Eng" },
    });
    await act(async () => {
      fireEvent.click(screen.getByTestId("project-form-save"));
    });

    expect(updateProjectSpy).toHaveBeenCalledTimes(1);
    const [id, req] = updateProjectSpy.mock.calls[0] as unknown as [
      string,
      { key: string; name: string; prompt_path: string | null; priority: number },
    ];
    expect(id).toBe("j-engine");
    expect(req.key).toBe("platform-eng");
    expect(req.name).toBe("Platform Eng");
    // Existing priority is preserved on edit.
    expect(req.priority).toBe(7);
    // The project-level update no longer carries statuses; the board's
    // per-status routes own them.
    expect(req).not.toHaveProperty("statuses");
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("disables Save and shows an error when the renamed key collides with another project", () => {
    projectsHookData = [
      makeProject(),
      {
        project_id: "j-other" as never,
        version: 1,
        project: {
          key: "platform" as never,
          name: "Platform",
          statuses: [],
          creator: "alice" as never,
          archived: false,
          prompt_path: null,
          priority: 0,
        },
      },
    ];
    render(
      <ProjectSettingsModal open onClose={() => {}} project={makeProject()} />,
    );

    fireEvent.change(screen.getByTestId("project-form-name"), {
      target: { value: "Platform" },
    });
    const save = screen.getByTestId("project-form-save") as HTMLButtonElement;
    expect(save.disabled).toBe(true);
    expect(screen.getByTestId("project-form-error").textContent).toContain(
      "already exists",
    );
  });

  it("writes the edited prompt document to the derived path on Save", async () => {
    getDocumentByPathSpy.mockImplementation(async () => ({
      document_id: "d-existing",
      version: 1,
      timestamp: "2026-01-01T00:00:00Z",
      document: {
        title: "t",
        body_markdown: "old",
        path: "/projects/engineering/prompt.md",
      },
    }));

    render(
      <ProjectSettingsModal open onClose={() => {}} project={makeProject()} />,
    );

    fireEvent.change(screen.getByTestId("project-form-prompt-body"), {
      target: { value: "new prompt body" },
    });
    await act(async () => {
      fireEvent.click(screen.getByTestId("project-form-save"));
    });

    expect(updateDocumentSpy).toHaveBeenCalledTimes(1);
    const docCall = updateDocumentSpy.mock.calls[0] as unknown as [
      string,
      { document: { body_markdown: string; path: string } },
    ];
    expect(docCall[1].document.body_markdown).toBe("new prompt body");
    expect(docCall[1].document.path).toBe("/projects/engineering/prompt.md");
    expect(updateProjectSpy).toHaveBeenCalledTimes(1);
  });

  it("Delete project confirms and deletes the project", async () => {
    const onClose = vi.fn();
    render(
      <ProjectSettingsModal open onClose={onClose} project={makeProject()} />,
    );

    fireEvent.click(screen.getByTestId("project-form-delete"));
    await act(async () => {
      fireEvent.click(screen.getByTestId("confirm-delete"));
    });

    expect(archiveProjectSpy).toHaveBeenCalledTimes(1);
    expect((archiveProjectSpy.mock.calls[0] as unknown as [string])[0]).toBe(
      "j-engine",
    );
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("invokes onClose when the modal close button is clicked", () => {
    const onClose = vi.fn();
    render(
      <ProjectSettingsModal open onClose={onClose} project={makeProject()} />,
    );
    fireEvent.click(screen.getByTestId("modal-close"));
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
