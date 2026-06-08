// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { useState } from "react";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";

const addToastSpy = vi.fn();
const invalidateQueriesSpy = vi.fn();

const getDocumentByPathSpy = vi.fn();
const createDocumentSpy = vi.fn();
const updateDocumentSpy = vi.fn();

// Track the most recent useQuery and useMutation configurations
// so each test can shape the response surface independently.
type QueryState = {
  isLoading: boolean;
  isError: boolean;
  isSuccess: boolean;
  data: unknown;
  error: Error | null;
};
let queryState: QueryState = {
  isLoading: false,
  isError: false,
  isSuccess: true,
  data: null,
  error: null,
};
const lastQueryFnRef: { fn: (() => unknown) | null } = { fn: null };

let mutationPending = false;

class ApiErrorMock extends Error {
  constructor(public readonly status: number, message: string) {
    super(message);
    this.name = "ApiError";
  }
}

vi.mock("../../../api/client", () => ({
  ApiError: ApiErrorMock,
  apiClient: {
    getDocumentByPath: getDocumentByPathSpy,
    createDocument: createDocumentSpy,
    updateDocument: updateDocumentSpy,
  },
}));

vi.mock("@tanstack/react-query", () => ({
  useQuery: ({ queryFn }: { queryKey: unknown; queryFn: () => unknown }) => {
    lastQueryFnRef.fn = queryFn;
    return queryState;
  },
  useMutation: ({
    mutationFn,
    onSuccess,
    onError,
  }: {
    mutationFn?: (vars: unknown) => Promise<unknown>;
    onSuccess?: (response: unknown, vars: unknown) => void;
    onError?: (err: Error) => void;
  }) => ({
    mutate: (vars: unknown) => {
      void (async () => {
        try {
          const result = await mutationFn?.(vars);
          onSuccess?.(result, vars);
        } catch (err) {
          onError?.(err as Error);
        }
      })();
    },
    isPending: mutationPending,
  }),
  useQueryClient: () => ({
    invalidateQueries: invalidateQueriesSpy,
  }),
}));

vi.mock("../../toast/useToast", () => ({
  useToast: () => ({ addToast: addToastSpy }),
}));

vi.mock("@hydra/ui", () => ({
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
    "data-testid"?: string;
  }) => (
    <label>
      {label}
      <input
        value={value}
        placeholder={placeholder}
        onChange={(e) =>
          onChange?.({ target: { value: e.target.value } })
        }
        data-testid={testId}
      />
    </label>
  ),
}));

vi.mock("../PromptDocumentEditor.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const { PromptDocumentEditor } = await import("../PromptDocumentEditor");

interface HarnessProps {
  initialPath?: string | null;
  defaultPath?: string;
  initiallyExpanded?: boolean;
}

function Harness({
  initialPath = null,
  defaultPath = "/projects/eng/prompt.md",
  initiallyExpanded = false,
}: HarnessProps) {
  const [path, setPath] = useState<string | null>(initialPath);
  const [expanded, setExpanded] = useState<boolean>(initiallyExpanded);
  return (
    <PromptDocumentEditor
      path={path}
      defaultPath={defaultPath}
      onPathChange={(p) => setPath(p)}
      expanded={expanded}
      onToggleExpanded={() => setExpanded((v) => !v)}
      testId="pde"
    />
  );
}

function resetState() {
  addToastSpy.mockReset();
  invalidateQueriesSpy.mockReset();
  getDocumentByPathSpy.mockReset();
  createDocumentSpy.mockReset();
  updateDocumentSpy.mockReset();
  mutationPending = false;
  queryState = {
    isLoading: false,
    isError: false,
    isSuccess: true,
    data: null,
    error: null,
  };
  lastQueryFnRef.fn = null;
}

describe("PromptDocumentEditor", () => {
  beforeEach(() => {
    resetState();
  });

  afterEach(() => {
    cleanup();
  });

  it("renders only the path input when collapsed and does not fetch", () => {
    render(<Harness />);

    // The path input is always present.
    expect(screen.getByTestId("pde")).toBeDefined();
    // The textarea and save button only mount when expanded.
    expect(screen.queryByTestId("pde-textarea")).toBeNull();
    expect(screen.queryByTestId("pde-save")).toBeNull();
    // Critically: no document fetch is triggered.
    expect(getDocumentByPathSpy).not.toHaveBeenCalled();
    expect(lastQueryFnRef.fn).toBeNull();
  });

  it("fetches and renders the loaded document body when expanded", async () => {
    queryState = {
      isLoading: false,
      isError: false,
      isSuccess: true,
      data: {
        document_id: "d-x",
        version: 1,
        timestamp: "2026-01-01",
        document: {
          title: "p",
          body_markdown: "loaded body",
        },
        creation_time: "2026-01-01",
      },
      error: null,
    };
    getDocumentByPathSpy.mockResolvedValue(queryState.data);

    render(
      <Harness
        initialPath="/projects/eng/prompt.md"
        initiallyExpanded
      />,
    );

    // The body should be wired up with the loaded markdown.
    const textarea = screen.getByTestId("pde-textarea") as HTMLTextAreaElement;
    expect(textarea.value).toBe("loaded body");

    // The query function should target the configured path.
    expect(lastQueryFnRef.fn).not.toBeNull();
    await act(async () => {
      await lastQueryFnRef.fn?.();
    });
    expect(getDocumentByPathSpy).toHaveBeenCalledWith(
      "/projects/eng/prompt.md",
    );
  });

  it("saves an existing document via updateDocument", async () => {
    const existing = {
      document_id: "d-existing",
      version: 1,
      timestamp: "2026-01-01",
      document: {
        title: "p",
        body_markdown: "old body",
        path: "/projects/eng/prompt.md",
      },
      creation_time: "2026-01-01",
    };
    queryState = {
      isLoading: false,
      isError: false,
      isSuccess: true,
      data: existing,
      error: null,
    };
    updateDocumentSpy.mockResolvedValue({
      document_id: "d-existing",
      version: 2,
    });

    render(
      <Harness
        initialPath="/projects/eng/prompt.md"
        initiallyExpanded
      />,
    );

    const textarea = screen.getByTestId("pde-textarea") as HTMLTextAreaElement;
    fireEvent.change(textarea, { target: { value: "new body" } });

    await act(async () => {
      fireEvent.click(screen.getByTestId("pde-save"));
    });

    expect(updateDocumentSpy).toHaveBeenCalledTimes(1);
    expect(updateDocumentSpy).toHaveBeenCalledWith("d-existing", {
      document: {
        ...existing.document,
        body_markdown: "new body",
        path: "/projects/eng/prompt.md",
      },
    });
    expect(createDocumentSpy).not.toHaveBeenCalled();
    expect(addToastSpy).toHaveBeenCalledWith("Prompt saved", "success");
    expect(invalidateQueriesSpy).toHaveBeenCalled();
  });

  it("creates a new document when no doc was loaded (404)", async () => {
    // Successful query that resolves to null — that's how the editor
    // signals "no document exists at this path yet".
    queryState = {
      isLoading: false,
      isError: false,
      isSuccess: true,
      data: null,
      error: null,
    };
    createDocumentSpy.mockResolvedValue({ document_id: "d-new", version: 1 });

    render(<Harness initialPath={null} initiallyExpanded />);

    const textarea = screen.getByTestId("pde-textarea") as HTMLTextAreaElement;
    fireEvent.change(textarea, { target: { value: "fresh body" } });

    await act(async () => {
      fireEvent.click(screen.getByTestId("pde-save"));
    });

    expect(createDocumentSpy).toHaveBeenCalledTimes(1);
    expect(createDocumentSpy).toHaveBeenCalledWith({
      document: {
        title: "/projects/eng/prompt.md",
        body_markdown: "fresh body",
        path: "/projects/eng/prompt.md",
      },
    });
    expect(updateDocumentSpy).not.toHaveBeenCalled();
    expect(addToastSpy).toHaveBeenCalledWith("Prompt saved", "success");
  });

  it("treats a 404 from getDocumentByPath as 'no document yet'", async () => {
    // Drive the wrapped queryFn directly to assert the 404 swallow.
    getDocumentByPathSpy.mockRejectedValue(
      new ApiErrorMock(404, "not found"),
    );
    render(<Harness initiallyExpanded />);
    expect(lastQueryFnRef.fn).not.toBeNull();
    const result = await lastQueryFnRef.fn?.();
    expect(result).toBeNull();

    // Other errors still bubble up.
    getDocumentByPathSpy.mockRejectedValue(new Error("boom"));
    await expect(lastQueryFnRef.fn?.()).rejects.toThrow("boom");
  });

  it("displays an inline error block when the load fails", () => {
    queryState = {
      isLoading: false,
      isError: true,
      isSuccess: false,
      data: null,
      error: new Error("network down"),
    };

    render(<Harness initiallyExpanded />);
    const errBlock = screen.getByTestId("pde-error");
    expect(errBlock.textContent).toContain("network down");
    expect(screen.queryByTestId("pde-textarea")).toBeNull();
  });

  it("displays an inline error block when the save fails", async () => {
    queryState = {
      isLoading: false,
      isError: false,
      isSuccess: true,
      data: null,
      error: null,
    };
    createDocumentSpy.mockRejectedValue(new Error("save boom"));

    render(<Harness initiallyExpanded />);

    const textarea = screen.getByTestId("pde-textarea") as HTMLTextAreaElement;
    fireEvent.change(textarea, { target: { value: "x" } });

    await act(async () => {
      fireEvent.click(screen.getByTestId("pde-save"));
    });

    expect(screen.getByTestId("pde-error").textContent).toContain("save boom");
    expect(addToastSpy).toHaveBeenCalledWith("save boom", "error");
  });

  it("propagates path-input edits via onPathChange", () => {
    render(<Harness />);
    const input = screen.getByTestId("pde") as HTMLInputElement;
    fireEvent.change(input, {
      target: { value: "/projects/eng/custom.md" },
    });
    // The controlled Harness re-renders with the new path:
    expect(
      (screen.getByTestId("pde") as HTMLInputElement).value,
    ).toBe("/projects/eng/custom.md");
  });

  it("toggles the body open/closed and only fetches once expanded", () => {
    render(<Harness />);
    expect(screen.queryByTestId("pde-textarea")).toBeNull();

    fireEvent.click(screen.getByTestId("pde-toggle"));
    expect(screen.getByTestId("pde-textarea")).toBeDefined();

    fireEvent.click(screen.getByTestId("pde-toggle"));
    expect(screen.queryByTestId("pde-textarea")).toBeNull();
  });

  it("disables Save while the draft is pristine", () => {
    queryState = {
      isLoading: false,
      isError: false,
      isSuccess: true,
      data: {
        document_id: "d-x",
        version: 1,
        timestamp: "2026-01-01",
        document: { title: "p", body_markdown: "pristine" },
        creation_time: "2026-01-01",
      },
      error: null,
    };
    render(<Harness initiallyExpanded />);
    const save = screen.getByTestId("pde-save") as HTMLButtonElement;
    expect(save.disabled).toBe(true);

    fireEvent.change(screen.getByTestId("pde-textarea"), {
      target: { value: "pristine + change" },
    });
    expect(save.disabled).toBe(false);
  });
});
