import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import { render, cleanup, fireEvent, screen } from "@testing-library/react";

vi.mock("@hydra/ui", () => ({
  Button: ({
    children,
    onClick,
    disabled,
    "aria-label": ariaLabel,
    title,
    className,
  }: {
    children: React.ReactNode;
    onClick?: () => void;
    disabled?: boolean;
    "aria-label"?: string;
    title?: string;
    className?: string;
  }) => (
    <button
      onClick={onClick}
      disabled={disabled}
      aria-label={ariaLabel}
      title={title}
      className={className}
    >
      {children}
    </button>
  ),
  Icons: {
    IconSend: () => <svg data-testid="icon-send" />,
  },
}));

vi.mock("../ChatInput.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const isMobileMock = vi.fn<() => boolean>(() => false);
vi.mock("../../../hooks/useIsMobile", () => ({
  useIsMobile: () => isMobileMock(),
  MOBILE_MEDIA_QUERY: "(max-width: 768px)",
}));

const { ChatInput } = await import("../ChatInput");
const { conversationDraftKey } = await import("../useConversationDraft");

function getTextarea(): HTMLTextAreaElement {
  return screen.getByPlaceholderText("Type a message…") as HTMLTextAreaElement;
}

function getSendButton(): HTMLButtonElement {
  return screen.getByRole("button", { name: "Send" }) as HTMLButtonElement;
}

describe("ChatInput keyboard shortcuts", () => {
  beforeEach(() => {
    window.localStorage.clear();
    isMobileMock.mockReturnValue(false);
  });

  afterEach(() => {
    cleanup();
    window.localStorage.clear();
    vi.clearAllMocks();
    isMobileMock.mockReturnValue(false);
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
    fireEvent.click(getSendButton());

    expect(onSend).toHaveBeenCalledTimes(1);
    expect(onSend).toHaveBeenCalledWith("hi there");
  });

  it("exposes the keyboard shortcut hint as a title tooltip on the Send button (desktop)", () => {
    render(<ChatInput conversationId="c-1" onSend={vi.fn()} />);
    expect(getSendButton().getAttribute("title")).toMatch(/to send/);
  });
});

describe("ChatInput on mobile", () => {
  beforeEach(() => {
    window.localStorage.clear();
    isMobileMock.mockReturnValue(true);
  });

  afterEach(() => {
    cleanup();
    window.localStorage.clear();
    vi.clearAllMocks();
    isMobileMock.mockReturnValue(false);
  });

  it("plain Enter does NOT call onSend on mobile", () => {
    const onSend = vi.fn();
    render(<ChatInput conversationId="c-1" onSend={onSend} />);
    const textarea = getTextarea();

    fireEvent.change(textarea, { target: { value: "hello world" } });
    fireEvent.keyDown(textarea, { key: "Enter" });

    expect(onSend).not.toHaveBeenCalled();
  });

  it("Send button still works on mobile", () => {
    const onSend = vi.fn();
    render(<ChatInput conversationId="c-1" onSend={onSend} />);
    const textarea = getTextarea();

    fireEvent.change(textarea, { target: { value: "hi" } });
    fireEvent.click(getSendButton());

    expect(onSend).toHaveBeenCalledWith("hi");
  });

  it("does not render the keyboard-shortcut tooltip on mobile", () => {
    render(<ChatInput conversationId="c-1" onSend={vi.fn()} />);
    expect(getSendButton().getAttribute("title")).toBeNull();
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

    fireEvent.click(getSendButton());

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

describe("ChatInput auto-grow", () => {
  beforeEach(() => {
    isMobileMock.mockReturnValue(false);
  });

  afterEach(() => {
    cleanup();
    window.localStorage.clear();
    vi.clearAllMocks();
    isMobileMock.mockReturnValue(false);
  });

  it("renders the textarea with a single visible row by default", () => {
    render(<ChatInput conversationId="c-1" onSend={vi.fn()} />);
    expect(getTextarea().getAttribute("rows")).toBe("1");
  });

  it("clamps the inline height to the desktop MIN when the value is empty", () => {
    render(<ChatInput conversationId="c-1" onSend={vi.fn()} />);
    // jsdom reports 0 for scrollHeight, so the layout effect should fall back
    // to MIN_HEIGHT_DESKTOP_PX (36px).
    expect(getTextarea().style.height).toBe("36px");
  });

  it("clamps the inline height to the mobile MIN when on mobile", () => {
    isMobileMock.mockReturnValue(true);
    render(<ChatInput conversationId="c-1" onSend={vi.fn()} />);
    // 28px Button.iconOnly bumps to 44px on touch viewports, so the textarea
    // floor bumps to MIN_HEIGHT_MOBILE_PX (52px) to fit it.
    expect(getTextarea().style.height).toBe("52px");
  });

  it("grows the inline height to scrollHeight + border when content is added", () => {
    render(<ChatInput conversationId="c-1" onSend={vi.fn()} />);
    const textarea = getTextarea();
    Object.defineProperty(textarea, "scrollHeight", { configurable: true, value: 120 });

    fireEvent.change(textarea, { target: { value: "line1\nline2\nline3\nline4" } });

    // 120 (scrollHeight = content + padding) + 2 (border-box border) = 122
    expect(textarea.style.height).toBe("122px");
  });

  it("clamps growth to the maximum height", () => {
    render(<ChatInput conversationId="c-1" onSend={vi.fn()} />);
    const textarea = getTextarea();
    Object.defineProperty(textarea, "scrollHeight", { configurable: true, value: 9999 });

    fireEvent.change(textarea, { target: { value: "a very tall message" } });

    expect(textarea.style.height).toBe("480px");
  });

  it("resets the height to MIN after sending", () => {
    const onSend = vi.fn();
    render(<ChatInput conversationId="c-1" onSend={onSend} />);
    const textarea = getTextarea();
    Object.defineProperty(textarea, "scrollHeight", { configurable: true, value: 120 });

    fireEvent.change(textarea, { target: { value: "tall content" } });
    expect(textarea.style.height).toBe("122px");

    Object.defineProperty(textarea, "scrollHeight", { configurable: true, value: 0 });
    fireEvent.click(getSendButton());

    expect(textarea.style.height).toBe("36px");
  });
});
