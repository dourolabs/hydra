// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";

// --- Mocks ---

const mockNavigate = vi.fn();
vi.mock("react-router-dom", () => ({
  useNavigate: () => mockNavigate,
}));

vi.mock("@tanstack/react-query", () => ({
  useMutation: ({
    mutationFn,
    onSuccess,
  }: {
    mutationFn: (input: unknown) => Promise<unknown>;
    onSuccess?: (data: unknown) => void;
  }) => ({
    mutate: (input: unknown) => {
      mutationFn(input).then((data) => {
        onSuccess?.(data);
      });
    },
    isPending: false,
  }),
  useQueryClient: () => ({ invalidateQueries: vi.fn() }),
}));

vi.mock("@hydra/ui", () => ({
  Avatar: ({ name }: { name: string }) => (
    <span data-testid={`avatar-${name}`} />
  ),
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
  Kbd: ({ children }: { children: ReactNode }) => <kbd>{children}</kbd>,
  Picker: ({
    label,
    value,
    open,
    onToggle,
    children,
    "data-testid": testId,
  }: {
    label: string;
    value: ReactNode;
    open: boolean;
    onToggle: () => void;
    children: ReactNode;
    "data-testid"?: string;
  }) => (
    <div data-testid={testId ?? `picker-${label.toLowerCase()}`}>
      <span>{label}</span>
      <button type="button" onClick={onToggle} aria-expanded={open} aria-label={label}>
        {value}
      </button>
      {open && <div data-testid={`picker-pop-${label.toLowerCase()}`}>{children}</div>}
    </div>
  ),
  PickerRow: ({
    onClick,
    children,
  }: {
    active?: boolean;
    onClick: () => void;
    children: ReactNode;
  }) => (
    <button type="button" onClick={onClick}>
      {children}
    </button>
  ),
  Icons: new Proxy(
    {},
    {
      get: (_t, prop) => () => <span data-testid={`icon-${String(prop)}`} />,
    },
  ),
}));

const createConversationMock = vi.fn<(body: unknown) => Promise<{ conversation_id: string }>>(
  () => Promise.resolve({ conversation_id: "c-new" }),
);
vi.mock("../../../api/client", () => ({
  apiClient: {
    createConversation: (body: unknown) => createConversationMock(body),
  },
}));

vi.mock("../../../hooks/useAgents", () => ({
  useAgents: () => ({
    data: [
      { name: "swe" },
      { name: "pm" },
      { name: "reviewer" },
    ],
  }),
}));

vi.mock("../../../hooks/useRepositories", () => ({
  useRepositories: () => ({
    data: [
      { name: "dourolabs/hydra" },
      { name: "dourolabs/aethon" },
    ],
  }),
}));

vi.mock("../../toast/useToast", () => ({
  useToast: () => ({ addToast: vi.fn() }),
}));

vi.mock("../ChatCreateModal.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

// --- Import after mocks ---
const { ChatCreateModal } = await import("../ChatCreateModal");
const { readChatCreateDefaults } = await import("../chatCreateDefaults");

const STORAGE_KEY = "hydra:v1:chat-create:defaults";

function openPickerByLabel(label: string) {
  const labelEl = screen.getByText(label);
  const wrapper = labelEl.parentElement!;
  const button = wrapper.querySelector("button")!;
  fireEvent.click(button);
}

describe("ChatCreateModal", () => {
  beforeEach(() => {
    localStorage.clear();
    createConversationMock.mockClear();
    createConversationMock.mockImplementation(() =>
      Promise.resolve({ conversation_id: "c-new" }),
    );
    mockNavigate.mockReset();
  });

  afterEach(() => {
    cleanup();
  });

  it("does not render when open=false", () => {
    render(<ChatCreateModal open={false} onClose={() => {}} />);
    expect(screen.queryByTestId("chat-create-modal")).toBeNull();
  });

  it("renders agent and repo pickers when open", () => {
    render(<ChatCreateModal open onClose={() => {}} />);
    expect(screen.getByTestId("chat-create-modal")).toBeDefined();
    expect(screen.getByTestId("chat-create-agent-picker")).toBeDefined();
    expect(screen.getByTestId("chat-create-repo-picker")).toBeDefined();
  });

  it("pre-fills the pickers from the persisted defaults on open", () => {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ agentName: "swe", repoName: "dourolabs/hydra" }),
    );

    const { rerender } = render(
      <ChatCreateModal open={false} onClose={() => {}} />,
    );
    rerender(<ChatCreateModal open onClose={() => {}} />);

    // Both pre-filled values should be visible in the trigger pills.
    expect(screen.getAllByText("swe").length).toBeGreaterThan(0);
    expect(screen.getAllByText("dourolabs/hydra").length).toBeGreaterThan(0);
  });

  it("posts agent_name + session_settings.repo_name when both are selected and navigates on success", async () => {
    createConversationMock.mockResolvedValueOnce({ conversation_id: "c-xyz" });
    const onClose = vi.fn();

    render(<ChatCreateModal open onClose={onClose} />);

    openPickerByLabel("Agent");
    fireEvent.click(screen.getByText("pm"));

    openPickerByLabel("Repository");
    fireEvent.click(screen.getByText("dourolabs/aethon"));

    fireEvent.click(screen.getByTestId("chat-create-submit"));

    await waitFor(() => {
      expect(createConversationMock).toHaveBeenCalledTimes(1);
    });

    expect(createConversationMock).toHaveBeenCalledWith({
      agent_name: "pm",
      session_settings: { repo_name: "dourolabs/aethon" },
    });

    await waitFor(() => {
      expect(mockNavigate).toHaveBeenCalledWith("/chat/c-xyz");
    });
    expect(onClose).toHaveBeenCalled();

    // localStorage persistence.
    expect(readChatCreateDefaults()).toEqual({
      agentName: "pm",
      repoName: "dourolabs/aethon",
    });
  });

  it("omits agent_name + session_settings when Unassigned / None are chosen", async () => {
    createConversationMock.mockResolvedValueOnce({ conversation_id: "c-empty" });
    // Seed with prior values to confirm Unassigned/None overwrite them.
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ agentName: "swe", repoName: "dourolabs/hydra" }),
    );

    const { rerender } = render(
      <ChatCreateModal open={false} onClose={() => {}} />,
    );
    rerender(<ChatCreateModal open onClose={() => {}} />);

    // Switch to Unassigned for the agent.
    openPickerByLabel("Agent");
    const unassignedRows = screen.getAllByText("Unassigned");
    // The "Unassigned" row in the popover — the first occurrence is the
    // trigger pill (since "swe" was pre-filled, but the trigger shows the
    // selection — so we use the picker popover's row).
    fireEvent.click(unassignedRows[unassignedRows.length - 1]);

    openPickerByLabel("Repository");
    const noneRows = screen.getAllByText("None");
    fireEvent.click(noneRows[noneRows.length - 1]);

    fireEvent.click(screen.getByTestId("chat-create-submit"));

    await waitFor(() => {
      expect(createConversationMock).toHaveBeenCalledWith({});
    });

    expect(readChatCreateDefaults()).toEqual({
      agentName: null,
      repoName: null,
    });
  });

  it("does not persist defaults when the user cancels", () => {
    const onClose = vi.fn();
    render(<ChatCreateModal open onClose={onClose} />);

    openPickerByLabel("Agent");
    fireEvent.click(screen.getByText("reviewer"));

    fireEvent.click(screen.getByText("Cancel"));

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(createConversationMock).not.toHaveBeenCalled();
    expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
  });

  it("closes without persisting on backdrop click", () => {
    const onClose = vi.fn();
    render(<ChatCreateModal open onClose={onClose} />);

    openPickerByLabel("Agent");
    fireEvent.click(screen.getByText("swe"));

    const backdrop = screen.getByTestId("chat-create-backdrop");
    fireEvent.click(backdrop);

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
  });

  it("closes on Escape", () => {
    const onClose = vi.fn();
    render(<ChatCreateModal open onClose={onClose} />);

    fireEvent.keyDown(document, { key: "Escape" });

    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
