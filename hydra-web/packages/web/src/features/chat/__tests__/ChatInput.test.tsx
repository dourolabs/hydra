import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
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
const { conversationDraftKey } = await import("../useConversationDraft");

function getTextarea(): HTMLTextAreaElement {
  return screen.getByPlaceholderText("Type a message…") as HTMLTextAreaElement;
}

describe("ChatInput keyboard shortcuts", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  afterEach(() => {
    cleanup();
    window.localStorage.clear();
    vi.clearAllMocks();
  });

  it("plain Enter triggers onSend with the trimmed typed value", () => {
    const onSend = vi.fn();
    render(<ChatInput conversationId="c-1" onSend={onSend} />);
    const textarea = getTextarea();

    fireEvent.change(textarea, { target: { value: "hello world" } });
    fireEvent.keyDown(textarea, { key: "Enter" });

    expect(onSend).toHaveBeenCalledTimes(1);
    expect(onSend).toHaveBeenCalledWith("hello world");
  });

  it("Shift+Enter does not call onSend (newline preserved)", () => {
    const onSend = vi.fn();
    render(<ChatInput conversationId="c-1" onSend={onSend} />);
    const textarea = getTextarea();

    fireEvent.change(textarea, { target: { value: "line1" } });
    fireEvent.keyDown(textarea, { key: "Enter", shiftKey: true });

    expect(onSend).not.toHaveBeenCalled();
  });

  it("Cmd+Enter does not submit", () => {
    const onSend = vi.fn();
    render(<ChatInput conversationId="c-1" onSend={onSend} />);
    const textarea = getTextarea();

    fireEvent.change(textarea, { target: { value: "hello" } });
    fireEvent.keyDown(textarea, { key: "Enter", metaKey: true });

    expect(onSend).not.toHaveBeenCalled();
  });

  it("Ctrl+Enter does not submit", () => {
    const onSend = vi.fn();
    render(<ChatInput conversationId="c-1" onSend={onSend} />);
    const textarea = getTextarea();

    fireEvent.change(textarea, { target: { value: "hello" } });
    fireEvent.keyDown(textarea, { key: "Enter", ctrlKey: true });

    expect(onSend).not.toHaveBeenCalled();
  });

  it("Enter with an empty value does not call onSend", () => {
    const onSend = vi.fn();
    render(<ChatInput conversationId="c-1" onSend={onSend} />);
    const textarea = getTextarea();

    fireEvent.keyDown(textarea, { key: "Enter" });

    expect(onSend).not.toHaveBeenCalled();
  });

  it("Enter with a whitespace-only value does not call onSend", () => {
    const onSend = vi.fn();
    render(<ChatInput conversationId="c-1" onSend={onSend} />);
    const textarea = getTextarea();

    fireEvent.change(textarea, { target: { value: "   \n  " } });
    fireEvent.keyDown(textarea, { key: "Enter" });

    expect(onSend).not.toHaveBeenCalled();
  });

  it("clicking the Send button sends the trimmed value", () => {
    const onSend = vi.fn();
    render(<ChatInput conversationId="c-1" onSend={onSend} />);
    const textarea = getTextarea();

    fireEvent.change(textarea, { target: { value: "  hi there  " } });
    fireEvent.click(screen.getByText("Send"));

    expect(onSend).toHaveBeenCalledTimes(1);
    expect(onSend).toHaveBeenCalledWith("hi there");
  });
});

describe("ChatInput draft persistence", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  afterEach(() => {
    cleanup();
    window.localStorage.clear();
    vi.clearAllMocks();
  });

  it("restores a draft from localStorage on mount", () => {
    window.localStorage.setItem(conversationDraftKey("c-1"), "saved draft");
    render(<ChatInput conversationId="c-1" onSend={vi.fn()} />);
    expect(getTextarea().value).toBe("saved draft");
  });

  it("persists typed text to localStorage as the user types", () => {
    render(<ChatInput conversationId="c-1" onSend={vi.fn()} />);
    fireEvent.change(getTextarea(), { target: { value: "work in progress" } });
    expect(window.localStorage.getItem(conversationDraftKey("c-1"))).toBe("work in progress");
  });

  it("removes the stored draft when the user clears the input", () => {
    window.localStorage.setItem(conversationDraftKey("c-1"), "old");
    render(<ChatInput conversationId="c-1" onSend={vi.fn()} />);
    fireEvent.change(getTextarea(), { target: { value: "" } });
    expect(window.localStorage.getItem(conversationDraftKey("c-1"))).toBeNull();
  });

  it("clears the draft after a successful send", () => {
    const onSend = vi.fn();
    window.localStorage.setItem(conversationDraftKey("c-1"), "queued message");
    render(<ChatInput conversationId="c-1" onSend={onSend} />);

    fireEvent.click(screen.getByText("Send"));

    expect(onSend).toHaveBeenCalledWith("queued message");
    expect(getTextarea().value).toBe("");
    expect(window.localStorage.getItem(conversationDraftKey("c-1"))).toBeNull();
  });

  it("keeps drafts per-conversation separate", () => {
    window.localStorage.setItem(conversationDraftKey("c-1"), "draft for one");
    window.localStorage.setItem(conversationDraftKey("c-2"), "draft for two");

    const { rerender } = render(<ChatInput conversationId="c-1" onSend={vi.fn()} />);
    expect(getTextarea().value).toBe("draft for one");

    rerender(<ChatInput conversationId="c-2" onSend={vi.fn()} />);
    expect(getTextarea().value).toBe("draft for two");

    // Writing in c-2 must not touch c-1's stored draft.
    fireEvent.change(getTextarea(), { target: { value: "updated two" } });
    expect(window.localStorage.getItem(conversationDraftKey("c-1"))).toBe("draft for one");
    expect(window.localStorage.getItem(conversationDraftKey("c-2"))).toBe("updated two");
  });
});
