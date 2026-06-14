// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import type { AgentRecord } from "@hydra/api";
import type { ReactNode } from "react";

const addToastSpy = vi.fn();
const invalidateQueriesSpy = vi.fn();

vi.mock("@tanstack/react-query", () => ({
  useMutation: ({
    mutationFn,
    onSuccess,
    onError,
  }: {
    mutationFn?: (input: unknown) => Promise<unknown>;
    onSuccess?: (response: unknown) => void;
    onError?: (err: Error) => void;
  }) => ({
    mutate: (input: unknown) => {
      void (async () => {
        try {
          const response = await mutationFn?.(input);
          onSuccess?.(response);
        } catch (err) {
          onError?.(err as Error);
        }
      })();
    },
    isPending: false,
  }),
  useQueryClient: () => ({
    invalidateQueries: invalidateQueriesSpy,
  }),
}));

vi.mock("@hydra/ui", () => ({
  Button: ({
    children,
    onClick,
    disabled,
  }: {
    children: ReactNode;
    onClick?: () => void;
    disabled?: boolean;
    variant?: string;
    size?: string;
  }) => (
    <button onClick={onClick} disabled={disabled}>
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
    "aria-label"?: string;
    "data-testid"?: string;
  }) => (
    <label>
      {label}
      <input
        type={type}
        value={value}
        disabled={disabled}
        onChange={(e) => onChange?.({ target: { value: e.target.value } })}
        placeholder={placeholder}
        aria-label={ariaLabel}
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
    hideLabel?: boolean;
    children: ReactNode;
  }) => (
    <div>
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
  Icons: {
    IconChevronRight: () => <span aria-hidden="true">▶</span>,
    IconChevronDown: () => <span aria-hidden="true">▼</span>,
  },
}));

const updateAgentSpy = vi.fn(
  async (_name: string, _req: unknown) => ({ agent: makeAgent() }),
);

vi.mock("../../../api/client", () => ({
  apiClient: {
    updateAgent: (name: string, req: unknown) => updateAgentSpy(name, req),
  },
}));

vi.mock("../../toast/useToast", () => ({
  useToast: () => ({ addToast: addToastSpy }),
}));

vi.mock("../../../hooks/useIsMobile", () => ({
  useIsMobile: () => false,
}));

vi.mock("../SecretsSelector", () => ({
  SecretsSelector: () => null,
}));

vi.mock("../AgentsSection.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("../AgentSessionSettingsFields.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("../../../components/SettingsSection/SettingsSection.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const { AgentEditModal } = await import("../AgentEditModal");

function makeAgent(overrides: Partial<AgentRecord> = {}): AgentRecord {
  return {
    name: "chat",
    prompt: "You are the chat agent.",
    prompt_path: "/agents/chat/prompt.md",
    mcp_config_path: null,
    mcp_config: null,
    max_tries: 3,
    max_simultaneous_interactive: 1,
    max_simultaneous_headless: 1,
    is_default_conversation_agent: false,
    secrets: [],
    ...overrides,
  };
}

describe("AgentEditModal — session_settings", () => {
  beforeEach(() => {
    addToastSpy.mockReset();
    invalidateQueriesSpy.mockReset();
    updateAgentSpy.mockClear();
  });

  afterEach(() => {
    cleanup();
  });

  function openSessionSettings() {
    fireEvent.click(
      screen.getByTestId("agent-edit-form-session-settings-toggle"),
    );
  }

  it("Save sends a non-default session_settings payload when fields are filled", async () => {
    render(
      <AgentEditModal
        open
        agent={makeAgent()}
        onClose={() => {}}
        agents={[makeAgent()]}
      />,
    );
    openSessionSettings();
    fireEvent.change(screen.getByTestId("agent-edit-form-cpu-limit"), {
      target: { value: "200m" },
    });
    fireEvent.change(screen.getByTestId("agent-edit-form-memory-limit"), {
      target: { value: "512Mi" },
    });
    await act(async () => {
      fireEvent.click(screen.getByText("Save Changes"));
    });

    expect(updateAgentSpy).toHaveBeenCalledTimes(1);
    const [name, req] = updateAgentSpy.mock.calls[0] as unknown as [
      string,
      { session_settings?: { cpu_limit?: string; memory_limit?: string } },
    ];
    expect(name).toBe("chat");
    expect(req.session_settings).toMatchObject({
      cpu_limit: "200m",
      memory_limit: "512Mi",
    });
  });

  it("collapses session_settings to undefined on Save when every subfield is empty", async () => {
    const seeded = makeAgent({
      session_settings: {
        cpu_limit: "500m",
        memory_limit: "1Gi",
      },
    });
    render(
      <AgentEditModal
        open
        agent={seeded}
        onClose={() => {}}
        agents={[seeded]}
      />,
    );
    openSessionSettings();
    fireEvent.change(screen.getByTestId("agent-edit-form-cpu-limit"), {
      target: { value: "" },
    });
    fireEvent.change(screen.getByTestId("agent-edit-form-memory-limit"), {
      target: { value: "" },
    });
    await act(async () => {
      fireEvent.click(screen.getByText("Save Changes"));
    });

    expect(updateAgentSpy).toHaveBeenCalledTimes(1);
    const [, req] = updateAgentSpy.mock.calls[0] as unknown as [
      string,
      { session_settings?: unknown },
    ];
    expect(req.session_settings).toBeUndefined();
  });

  it("prefills inputs from the agent's existing session_settings", () => {
    const seeded = makeAgent({
      session_settings: {
        cpu_limit: "500m",
        memory_limit: "1Gi",
        image: "ghcr.io/org/img:v1",
        model: "claude-opus-4-7",
        max_retries: 3,
        idle_timeout: {
          kind: "seconds",
          value: 600 as unknown as bigint,
        },
      },
    });
    render(
      <AgentEditModal
        open
        agent={seeded}
        onClose={() => {}}
        agents={[seeded]}
      />,
    );
    openSessionSettings();
    expect(
      (screen.getByTestId("agent-edit-form-cpu-limit") as HTMLInputElement)
        .value,
    ).toBe("500m");
    expect(
      (screen.getByTestId("agent-edit-form-memory-limit") as HTMLInputElement)
        .value,
    ).toBe("1Gi");
    expect(
      (screen.getByTestId("agent-edit-form-image") as HTMLInputElement).value,
    ).toBe("ghcr.io/org/img:v1");
    expect(
      (screen.getByTestId("agent-edit-form-model") as HTMLInputElement).value,
    ).toBe("claude-opus-4-7");
    expect(
      (screen.getByTestId("agent-edit-form-max-retries") as HTMLInputElement)
        .value,
    ).toBe("3");
    expect(
      (
        screen.getByTestId(
          "agent-edit-form-idle-timeout-seconds",
        ) as HTMLInputElement
      ).value,
    ).toBe("600");
  });
});
