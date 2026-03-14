import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import React from "react";

// --- Mocks ---

let mockSecrets: string[] = ["OPENAI_API_KEY"];
let mockIsLoading = false;
let mockError: Error | null = null;

vi.mock("../../auth/useUsername", () => ({
  useUsername: () => "test-user",
}));

vi.mock("../../secrets/useSecrets", () => ({
  useSecrets: () => ({
    data: mockError ? undefined : { secrets: mockSecrets },
    isLoading: mockIsLoading,
    error: mockError,
  }),
}));

const mockAddToast = vi.fn();
vi.mock("../../toast/useToast", () => ({
  useToast: () => ({ addToast: mockAddToast }),
}));

let mutationSuccess = true;

vi.mock("@tanstack/react-query", () => ({
  useMutation: ({ onSuccess, onError }: {
    mutationFn: (args: unknown) => Promise<unknown>;
    onSuccess?: (data: unknown, variables: unknown) => void;
    onError?: (err: Error) => void;
  }) => ({
    mutate: (...args: unknown[]) => {
      if (mutationSuccess) {
        onSuccess?.(undefined, args[0]);
      } else {
        onError?.(new Error("Mutation failed"));
      }
    },
    isPending: false,
  }),
  useQueryClient: () => ({ invalidateQueries: vi.fn() }),
}));

vi.mock("../../../api/client", () => ({
  apiClient: {
    setSecret: vi.fn(),
    deleteSecret: vi.fn(),
  },
}));

vi.mock("@metis/ui", () => ({
  Panel: ({ children }: { children: React.ReactNode }) => <div data-testid="panel">{children}</div>,
  Spinner: () => <div data-testid="spinner" />,
  Button: ({ children, onClick, disabled, ...rest }: {
    children: React.ReactNode;
    onClick?: () => void;
    disabled?: boolean;
    variant?: string;
    size?: string;
  }) => (
    <button onClick={onClick} disabled={disabled} {...rest}>
      {children}
    </button>
  ),
  Input: ({ value, onChange, placeholder, type, autoFocus }: {
    value?: string;
    onChange?: (e: React.ChangeEvent<HTMLInputElement>) => void;
    placeholder?: string;
    type?: string;
    autoFocus?: boolean;
  }) => (
    <input
      value={value}
      onChange={onChange}
      placeholder={placeholder}
      type={type}
      autoFocus={autoFocus}
      data-testid={`input-${placeholder?.replace(/\s+/g, "-").toLowerCase()}`}
    />
  ),
}));

vi.mock("../../secrets/SecretsSection.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

// --- Import after mocks ---
const { SecretsSection } = await import("../../secrets/SecretsSection");

// --- Tests ---

describe("SecretsSection", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockSecrets = ["OPENAI_API_KEY"];
    mockIsLoading = false;
    mockError = null;
    mutationSuccess = true;
  });

  it("renders section title", () => {
    render(<SecretsSection />);
    expect(screen.getByText("Secrets")).toBeDefined();
  });

  it("renders known secrets", () => {
    render(<SecretsSection />);
    expect(screen.getByText("GH_TOKEN")).toBeDefined();
    expect(screen.getByText("OPENAI_API_KEY")).toBeDefined();
    expect(screen.getByText("ANTHROPIC_API_KEY")).toBeDefined();
    expect(screen.getByText("CLAUDE_CODE_OAUTH_TOKEN")).toBeDefined();
  });

  it("renders GH_TOKEN with auto-provided description", () => {
    render(<SecretsSection />);
    expect(screen.getByText("GitHub Token")).toBeDefined();
    expect(screen.getByText("Automatically provided from your GitHub login. You can override it below if needed.")).toBeDefined();
  });

  it("shows Configured for configured secrets", () => {
    render(<SecretsSection />);
    expect(screen.getAllByText("Configured").length).toBeGreaterThan(0);
  });

  it("shows Not set for unconfigured secrets", () => {
    render(<SecretsSection />);
    expect(screen.getAllByText("Not set").length).toBeGreaterThan(0);
  });

  it("shows Update button for configured secrets, Set for unconfigured", () => {
    render(<SecretsSection />);
    expect(screen.getAllByText("Update").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Set").length).toBeGreaterThan(0);
  });

  it("shows Delete button only for configured secrets", () => {
    render(<SecretsSection />);
    const deleteButtons = screen.getAllByText("Delete");
    // Only OPENAI_API_KEY is configured
    expect(deleteButtons.length).toBe(1);
  });

  it("shows spinner when loading", () => {
    mockIsLoading = true;
    render(<SecretsSection />);
    expect(screen.getByTestId("spinner")).toBeDefined();
  });

  it("shows error message on fetch error", () => {
    mockError = new Error("Network error");
    render(<SecretsSection />);
    expect(screen.getByText(/Failed to load secrets/)).toBeDefined();
    expect(screen.getByText(/Network error/)).toBeDefined();
  });

  it("opens edit form on Set/Update click", () => {
    render(<SecretsSection />);
    const setButtons = screen.getAllByText("Set");
    fireEvent.click(setButtons[0]);
    // Should now show Cancel and Save buttons
    expect(screen.getByText("Cancel")).toBeDefined();
    expect(screen.getByText("Save")).toBeDefined();
  });

  it("closes edit form on Cancel click", () => {
    render(<SecretsSection />);
    const setButtons = screen.getAllByText("Set");
    fireEvent.click(setButtons[0]);
    fireEvent.click(screen.getByText("Cancel"));
    // Cancel and Save should be gone; Set should be back
    expect(screen.queryByText("Cancel")).toBeNull();
  });

  it("shows toast on successful save", () => {
    render(<SecretsSection />);
    const updateButtons = screen.getAllByText("Update");
    fireEvent.click(updateButtons[0]);

    const input = screen.getByPlaceholderText("Enter OPENAI_API_KEY") as HTMLInputElement;
    fireEvent.change(input, { target: { value: "sk-test123" } });
    fireEvent.click(screen.getByText("Save"));

    expect(mockAddToast).toHaveBeenCalledWith("OPENAI_API_KEY saved", "success");
  });

  it("shows toast on delete", () => {
    render(<SecretsSection />);
    const deleteButtons = screen.getAllByText("Delete");
    fireEvent.click(deleteButtons[0]);
    expect(mockAddToast).toHaveBeenCalledWith("OPENAI_API_KEY deleted", "success");
  });

  it("shows toast on mutation error", () => {
    mutationSuccess = false;
    render(<SecretsSection />);
    const deleteButtons = screen.getAllByText("Delete");
    fireEvent.click(deleteButtons[0]);
    expect(mockAddToast).toHaveBeenCalledWith("Mutation failed", "error");
  });

  it("does not save empty value", () => {
    render(<SecretsSection />);
    const updateButtons = screen.getAllByText("Update");
    fireEvent.click(updateButtons[0]);
    // Save button should be disabled when input is empty
    const saveBtn = screen.getByText("Save") as HTMLButtonElement;
    expect(saveBtn.disabled).toBe(true);
  });

  it("closes edit form on Escape key", () => {
    render(<SecretsSection />);
    const setButtons = screen.getAllByText("Set");
    fireEvent.click(setButtons[0]);
    const form = screen.getByText("Cancel").closest("div")!;
    fireEvent.keyDown(form, { key: "Escape" });
    expect(screen.queryByText("Cancel")).toBeNull();
  });

  it("renders custom secrets", () => {
    mockSecrets = ["OPENAI_API_KEY", "MY_CUSTOM_SECRET"];
    render(<SecretsSection />);
    expect(screen.getByText("MY_CUSTOM_SECRET")).toBeDefined();
    expect(screen.getByText("Custom secret")).toBeDefined();
  });

  // --- AddSecretForm tests ---

  it("shows Add Secret button", () => {
    render(<SecretsSection />);
    expect(screen.getByText("+ Add Secret")).toBeDefined();
  });

  it("opens add form on Add Secret click", () => {
    render(<SecretsSection />);
    fireEvent.click(screen.getByText("+ Add Secret"));
    expect(screen.getByPlaceholderText("SECRET_NAME")).toBeDefined();
    expect(screen.getByPlaceholderText("Secret value")).toBeDefined();
  });

  it("validates secret name format", () => {
    render(<SecretsSection />);
    fireEvent.click(screen.getByText("+ Add Secret"));

    const nameInput = screen.getByPlaceholderText("SECRET_NAME") as HTMLInputElement;
    fireEvent.change(nameInput, { target: { value: "1BAD" } });

    expect(screen.getByText("Must be 1-128 chars, start with uppercase letter, only uppercase letters/digits/underscores")).toBeDefined();
  });

  it("validates METIS_ prefix is reserved", () => {
    render(<SecretsSection />);
    fireEvent.click(screen.getByText("+ Add Secret"));

    const nameInput = screen.getByPlaceholderText("SECRET_NAME") as HTMLInputElement;
    fireEvent.change(nameInput, { target: { value: "METIS_FOO" } });

    expect(screen.getByText("Names starting with METIS_ are reserved")).toBeDefined();
  });

  it("validates duplicate name on save", () => {
    mockSecrets = ["OPENAI_API_KEY", "MY_SECRET"];
    render(<SecretsSection />);
    fireEvent.click(screen.getByText("+ Add Secret"));

    const nameInput = screen.getByPlaceholderText("SECRET_NAME") as HTMLInputElement;
    fireEvent.change(nameInput, { target: { value: "MY_SECRET" } });

    const valueInput = screen.getByPlaceholderText("Secret value") as HTMLInputElement;
    fireEvent.change(valueInput, { target: { value: "some-value" } });

    // Find the Save button in the add form (not in a secret row form)
    const saveButtons = screen.getAllByText("Save");
    fireEvent.click(saveButtons[saveButtons.length - 1]);

    expect(screen.getByText(/already exists/)).toBeDefined();
  });

  it("closes add form on Cancel", () => {
    render(<SecretsSection />);
    fireEvent.click(screen.getByText("+ Add Secret"));
    expect(screen.getByPlaceholderText("SECRET_NAME")).toBeDefined();

    const cancelButtons = screen.getAllByText("Cancel");
    fireEvent.click(cancelButtons[cancelButtons.length - 1]);
    expect(screen.queryByPlaceholderText("SECRET_NAME")).toBeNull();
  });

  it("closes add form on Escape", () => {
    render(<SecretsSection />);
    fireEvent.click(screen.getByText("+ Add Secret"));
    const form = screen.getByPlaceholderText("SECRET_NAME").closest("div")!;
    fireEvent.keyDown(form, { key: "Escape" });
    expect(screen.queryByPlaceholderText("SECRET_NAME")).toBeNull();
  });
});
