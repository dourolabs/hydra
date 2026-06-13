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

const mutateSpy = vi.fn();
const addToastSpy = vi.fn();
let mutationPending = false;
let simulateError: Error | null = null;
const cancelQueriesSpy = vi.fn(async () => {});
const setQueryDataSpy = vi.fn();
const invalidateQueriesSpy = vi.fn();
const navigateSpy = vi.fn();
let queryDataByKey: Map<string, unknown> = new Map();
let projectsHookData: ProjectRecord[] | undefined = [];

vi.mock("@tanstack/react-query", () => ({
  // Prompt-doc loader (usePromptDocumentBody) calls useQuery; create mode never
  // loads an existing doc, so a neutral success state seeds an empty body.
  useQuery: () => ({
    isLoading: false,
    isError: false,
    isSuccess: true,
    data: null,
    error: null,
  }),
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
      mutateSpy();
      void (async () => {
        const ctx = onMutate ? await Promise.resolve(onMutate()) : undefined;
        if (simulateError) {
          onError?.(simulateError, undefined, ctx);
          return;
        }
        try {
          const response = (await mutationFn?.()) ?? { project_id: "j-new" };
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
    getQueryData: (key: readonly unknown[]) =>
      queryDataByKey.get(JSON.stringify(key)),
    setQueryData: (key: readonly unknown[], value: unknown) => {
      queryDataByKey.set(JSON.stringify(key), value);
      setQueryDataSpy(key, value);
    },
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
      <div role="dialog" aria-label={title} data-testid="modal">
        <h2>{title}</h2>
        {children}
      </div>
    ) : null,
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
        onChange={(e) => onChange?.({ target: { value: e.target.value } })}
        placeholder={placeholder}
        data-testid={testId}
      />
    </label>
  ),
}));

const createDocumentSpy = vi.fn<
  (req: unknown) => Promise<{ document_id: string }>
>(async () => ({ document_id: "d-1" }));
const createProjectSpy = vi.fn<
  (req: unknown) => Promise<{ project_id: string; version: bigint }>
>(async () => ({ project_id: "j-new", version: 1n }));
vi.mock("../../../api/client", () => ({
  apiClient: {
    createDocument: createDocumentSpy,
    createProject: createProjectSpy,
  },
}));

vi.mock("../../toast/useToast", () => ({
  useToast: () => ({ addToast: addToastSpy }),
}));

vi.mock("../../auth/useUsername", () => ({
  useUsername: () => "alice",
}));

vi.mock("../useProjects", () => ({
  useProjects: () => ({ data: projectsHookData }),
}));

vi.mock("../ProjectForm.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("../../../components/DeleteConfirmModal/DeleteConfirmModal", () => ({
  DeleteConfirmModal: () => null,
}));

const { ProjectCreateModal } = await import("../ProjectCreateModal");

function makeProject(key: string, name: string): ProjectRecord {
  return {
    project_id: `j-${key}` as never,
    version: 1,
    project: {
      key: key as never,
      name,
      statuses: [],
      creator: "alice" as never,
      archived: false,
      prompt_path: null,
      priority: 0,
    },
  };
}

describe("ProjectCreateModal", () => {
  beforeEach(() => {
    mutateSpy.mockReset();
    addToastSpy.mockReset();
    cancelQueriesSpy.mockClear();
    setQueryDataSpy.mockReset();
    invalidateQueriesSpy.mockReset();
    navigateSpy.mockReset();
    createDocumentSpy.mockClear();
    createProjectSpy.mockClear();
    mutationPending = false;
    simulateError = null;
    queryDataByKey = new Map();
    projectsHookData = [];
  });

  afterEach(() => {
    cleanup();
  });

  it("renders only Name + Prompt inputs (no key field, no status list, no prompt-path field)", () => {
    render(<ProjectCreateModal open={true} onClose={() => {}} />);
    expect(screen.getByTestId("project-form-name")).toBeDefined();
    expect(screen.getByTestId("project-form-prompt-body")).toBeDefined();
    // The simplified modal must not expose any of the old form fields.
    expect(screen.queryByTestId("project-editor-key")).toBeNull();
    expect(screen.queryByTestId("project-editor-prompt-path")).toBeNull();
    expect(screen.queryByTestId("project-editor-add-status")).toBeNull();
  });

  it("disables Save until the user types a name with at least one slug-able character", () => {
    render(<ProjectCreateModal open={true} onClose={() => {}} />);
    const save = screen.getByTestId("project-form-save") as HTMLButtonElement;
    expect(save.disabled).toBe(true);

    fireEvent.change(screen.getByTestId("project-form-name"), {
      target: { value: "@@@" },
    });
    expect(save.disabled).toBe(true);
    expect(screen.getByTestId("project-form-error").textContent).toContain(
      "letter or digit",
    );

    fireEvent.change(screen.getByTestId("project-form-name"), {
      target: { value: "Engineering" },
    });
    expect(save.disabled).toBe(false);
    expect(screen.queryByTestId("project-form-error")).toBeNull();
  });

  it("disables Save and shows error when the derived key collides with an existing project", () => {
    projectsHookData = [makeProject("engineering", "Engineering")];
    render(<ProjectCreateModal open={true} onClose={() => {}} />);

    fireEvent.change(screen.getByTestId("project-form-name"), {
      target: { value: "Engineering" },
    });

    const save = screen.getByTestId("project-form-save") as HTMLButtonElement;
    expect(save.disabled).toBe(true);
    expect(screen.getByTestId("project-form-error").textContent).toContain(
      "already exists",
    );
  });

  it("writes the prompt document at /projects/<key>/prompt.md, then creates the project with empty statuses", async () => {
    render(<ProjectCreateModal open={true} onClose={() => {}} />);
    fireEvent.change(screen.getByTestId("project-form-name"), {
      target: { value: "Engineering" },
    });
    fireEvent.change(screen.getByTestId("project-form-prompt-body"), {
      target: { value: "Hello prompt" },
    });

    await act(async () => {
      fireEvent.click(screen.getByTestId("project-form-save"));
    });

    expect(createDocumentSpy).toHaveBeenCalledTimes(1);
    const [docPayload] = createDocumentSpy.mock.calls[0] as unknown as [
      { document: { path: string; body_markdown: string; title: string } },
    ];
    expect(docPayload.document.path).toBe("/projects/engineering/prompt.md");
    expect(docPayload.document.body_markdown).toBe("Hello prompt");
    expect(docPayload.document.title).toBe("/projects/engineering/prompt.md");

    expect(createProjectSpy).toHaveBeenCalledTimes(1);
    const [projectPayload] = createProjectSpy.mock.calls[0] as unknown as [
      {
        key: string;
        name: string;
        prompt_path: string | null;
        priority: number;
      },
    ];
    expect(projectPayload.key).toBe("engineering");
    expect(projectPayload.name).toBe("Engineering");
    expect(projectPayload.prompt_path).toBe("/projects/engineering/prompt.md");
    expect(projectPayload.priority).toBe(0);
  });

  it("skips the document write when the prompt body is blank", async () => {
    render(<ProjectCreateModal open={true} onClose={() => {}} />);
    fireEvent.change(screen.getByTestId("project-form-name"), {
      target: { value: "Engineering" },
    });

    await act(async () => {
      fireEvent.click(screen.getByTestId("project-form-save"));
    });

    expect(createDocumentSpy).not.toHaveBeenCalled();
    expect(createProjectSpy).toHaveBeenCalledTimes(1);
  });

  it("closes the modal on success without navigating away from the originating page", async () => {
    const onClose = vi.fn();
    render(<ProjectCreateModal open={true} onClose={onClose} />);
    fireEvent.change(screen.getByTestId("project-form-name"), {
      target: { value: "Engineering" },
    });

    await act(async () => {
      fireEvent.click(screen.getByTestId("project-form-save"));
    });

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(navigateSpy).not.toHaveBeenCalled();
  });

  it("slugifies multi-word names to a kebab-case project key", async () => {
    render(<ProjectCreateModal open={true} onClose={() => {}} />);
    fireEvent.change(screen.getByTestId("project-form-name"), {
      target: { value: "Growth Team" },
    });

    await act(async () => {
      fireEvent.click(screen.getByTestId("project-form-save"));
    });

    const [projectPayload] = createProjectSpy.mock.calls[0] as unknown as [
      { key: string; prompt_path: string | null },
    ];
    expect(projectPayload.key).toBe("growth-team");
    expect(projectPayload.prompt_path).toBe("/projects/growth-team/prompt.md");
  });
});
