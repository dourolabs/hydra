import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, cleanup } from "@testing-library/react";
import type { ConversationEvent } from "@hydra/api";

vi.mock("@hydra/ui", () => ({
  Tooltip: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  MarkdownViewer: ({ content }: { content: string }) => <div>{content}</div>,
}));

vi.mock("../../../utils/time", () => ({
  formatTimestamp: (s: string) => s,
  formatRelativeTime: (s: string) => s,
}));

vi.mock("../ChatMessageList.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const { ChatMessageList } = await import("../ChatMessageList");

function userMessage(content: string, ts = "2026-05-14T00:00:00Z"): ConversationEvent {
  return { type: "user_message", content, timestamp: ts };
}

describe("ChatMessageList auto-scroll", () => {
  let scrollIntoViewSpy: ReturnType<typeof vi.fn>;
  let originalScrollIntoView: typeof Element.prototype.scrollIntoView | undefined;

  beforeEach(() => {
    originalScrollIntoView = Element.prototype.scrollIntoView;
    scrollIntoViewSpy = vi.fn();
    Element.prototype.scrollIntoView = scrollIntoViewSpy;
  });

  afterEach(() => {
    cleanup();
    if (originalScrollIntoView) {
      Element.prototype.scrollIntoView = originalScrollIntoView;
    }
    vi.clearAllMocks();
  });

  it("calls scrollIntoView on initial mount", () => {
    render(<ChatMessageList events={[userMessage("hello")]} />);
    expect(scrollIntoViewSpy).toHaveBeenCalled();
    expect(scrollIntoViewSpy).toHaveBeenCalledWith({ behavior: "smooth" });
  });

  it("calls scrollIntoView again when events grows", () => {
    const { rerender } = render(<ChatMessageList events={[userMessage("hi")]} />);
    scrollIntoViewSpy.mockClear();

    rerender(
      <ChatMessageList events={[userMessage("hi"), userMessage("hello")]} />,
    );

    expect(scrollIntoViewSpy).toHaveBeenCalledTimes(1);
    expect(scrollIntoViewSpy).toHaveBeenCalledWith({ behavior: "smooth" });
  });

  it("does not call scrollIntoView again when events length is unchanged", () => {
    const events = [userMessage("hi"), userMessage("there")];
    const { rerender } = render(<ChatMessageList events={events} />);
    scrollIntoViewSpy.mockClear();

    // Re-render with a new array reference but same length.
    rerender(<ChatMessageList events={[...events]} />);

    expect(scrollIntoViewSpy).not.toHaveBeenCalled();
  });
});
