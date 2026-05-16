import { describe, it, expect, vi, afterEach } from "vitest";
import { render, cleanup, fireEvent, screen } from "@testing-library/react";

vi.mock("@hydra/ui", () => ({
  Button: ({
    children,
    onClick,
    disabled,
  }: {
    children: React.ReactNode;
    onClick?: () => void;
    disabled?: boolean;
  }) => (
    <button onClick={onClick} disabled={disabled}>
      {children}
    </button>
  ),
  Kbd: ({ children }: { children: React.ReactNode }) => <kbd>{children}</kbd>,
}));

vi.mock("../ChatInput.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const { ChatInput } = await import("../ChatInput");

function getTextarea(): HTMLTextAreaElement {
  return screen.getByPlaceholderText("Type a message…") as HTMLTextAreaElement;
}

describe("ChatInput keyboard shortcuts", () => {
  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
  });

  it("plain Enter triggers onSend with the trimmed typed value", () => {
    const onSend = vi.fn();
    render(<ChatInput onSend={onSend} />);
    const textarea = getTextarea();

    fireEvent.change(textarea, { target: { value: "hello world" } });
    fireEvent.keyDown(textarea, { key: "Enter" });

    expect(onSend).toHaveBeenCalledTimes(1);
    expect(onSend).toHaveBeenCalledWith("hello world");
  });

  it("Shift+Enter does not call onSend (newline preserved)", () => {
    const onSend = vi.fn();
    render(<ChatInput onSend={onSend} />);
    const textarea = getTextarea();

    fireEvent.change(textarea, { target: { value: "line1" } });
    fireEvent.keyDown(textarea, { key: "Enter", shiftKey: true });

    expect(onSend).not.toHaveBeenCalled();
  });

  it("Cmd+Enter does not submit", () => {
    const onSend = vi.fn();
    render(<ChatInput onSend={onSend} />);
    const textarea = getTextarea();

    fireEvent.change(textarea, { target: { value: "hello" } });
    fireEvent.keyDown(textarea, { key: "Enter", metaKey: true });

    expect(onSend).not.toHaveBeenCalled();
  });

  it("Ctrl+Enter does not submit", () => {
    const onSend = vi.fn();
    render(<ChatInput onSend={onSend} />);
    const textarea = getTextarea();

    fireEvent.change(textarea, { target: { value: "hello" } });
    fireEvent.keyDown(textarea, { key: "Enter", ctrlKey: true });

    expect(onSend).not.toHaveBeenCalled();
  });

  it("Enter with an empty value does not call onSend", () => {
    const onSend = vi.fn();
    render(<ChatInput onSend={onSend} />);
    const textarea = getTextarea();

    fireEvent.keyDown(textarea, { key: "Enter" });

    expect(onSend).not.toHaveBeenCalled();
  });

  it("Enter with a whitespace-only value does not call onSend", () => {
    const onSend = vi.fn();
    render(<ChatInput onSend={onSend} />);
    const textarea = getTextarea();

    fireEvent.change(textarea, { target: { value: "   \n  " } });
    fireEvent.keyDown(textarea, { key: "Enter" });

    expect(onSend).not.toHaveBeenCalled();
  });

  it("clicking the Send button sends the trimmed value", () => {
    const onSend = vi.fn();
    render(<ChatInput onSend={onSend} />);
    const textarea = getTextarea();

    fireEvent.change(textarea, { target: { value: "  hi there  " } });
    fireEvent.click(screen.getByText("Send"));

    expect(onSend).toHaveBeenCalledTimes(1);
    expect(onSend).toHaveBeenCalledWith("hi there");
  });
});
