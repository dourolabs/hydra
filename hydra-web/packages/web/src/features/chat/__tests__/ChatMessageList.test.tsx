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
  shortRelativeTime: (s: string) => s,
}));

vi.mock("../ChatMessageList.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const { ChatMessageList } = await import("../ChatMessageList");

function userMessage(content: string, ts = "2026-05-14T00:00:00Z"): ConversationEvent {
  return { type: "user_message", content, timestamp: ts };
}

describe("ChatMessageList auto-scroll", () => {
  let scrollToSpy: ReturnType<typeof vi.fn>;
  let originalScrollTo: typeof Element.prototype.scrollTo | undefined;

  beforeEach(() => {
    originalScrollTo = Element.prototype.scrollTo;
    scrollToSpy = vi.fn();
    Element.prototype.scrollTo = scrollToSpy as unknown as typeof Element.prototype.scrollTo;
  });

  afterEach(() => {
    cleanup();
    if (originalScrollTo) {
      Element.prototype.scrollTo = originalScrollTo;
    }
    vi.clearAllMocks();
  });

  it("scrolls the container to bottom on initial mount", () => {
    render(<ChatMessageList events={[userMessage("hello")]} />);
    expect(scrollToSpy).toHaveBeenCalled();
    const call = scrollToSpy.mock.calls[0]?.[0];
    expect(call).toMatchObject({ behavior: "smooth" });
    expect(typeof call.top).toBe("number");
  });

  it("scrolls the container to bottom when events grows", () => {
    const { rerender } = render(<ChatMessageList events={[userMessage("hi")]} />);
    scrollToSpy.mockClear();

    rerender(<ChatMessageList events={[userMessage("hi"), userMessage("hello")]} />);

    expect(scrollToSpy).toHaveBeenCalledTimes(1);
    expect(scrollToSpy.mock.calls[0]?.[0]).toMatchObject({ behavior: "smooth" });
  });

  it("does not scroll again when events length is unchanged", () => {
    const events = [userMessage("hi"), userMessage("there")];
    const { rerender } = render(<ChatMessageList events={events} />);
    scrollToSpy.mockClear();

    rerender(<ChatMessageList events={[...events]} />);

    expect(scrollToSpy).not.toHaveBeenCalled();
  });
});
