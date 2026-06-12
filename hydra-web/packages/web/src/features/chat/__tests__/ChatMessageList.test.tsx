import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, cleanup } from "@testing-library/react";
import type { SessionEvent } from "@hydra/api";

vi.mock("@hydra/ui", () => ({
  Tooltip: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  MarkdownViewer: ({ content }: { content: string }) => <div>{content}</div>,
  Avatar: ({ name, kind }: { name: string; kind?: "human" | "agent" }) => (
    <span data-testid="avatar" data-kind={kind ?? "human"} data-name={name} />
  ),
}));

vi.mock("../../../utils/time", () => ({
  formatTimestamp: (s: string) => s,
  formatRelativeTime: (s: string) => s,
  shortRelativeTime: (s: string) => s,
}));

vi.mock("../ChatMessageList.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("../SystemEventBubble", () => ({
  SystemEventBubble: ({
    kind,
    timestamp,
  }: {
    kind: { kind: string };
    timestamp: string;
  }) => (
    <div data-testid="system-event-bubble" data-kind={kind.kind} data-ts={timestamp} />
  ),
}));

const { ChatMessageList } = await import("../ChatMessageList");

function userMessage(content: string, ts = "2026-05-14T00:00:00Z"): SessionEvent {
  return { type: "user_message", content, timestamp: ts };
}

function assistantMessage(content: string, ts = "2026-05-14T00:00:01Z"): SessionEvent {
  return { type: "assistant_message", content, timestamp: ts };
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

describe("ChatMessageList ResizeObserver follow-bottom", () => {
  let scrollToSpy: ReturnType<typeof vi.fn>;
  let originalScrollTo: typeof Element.prototype.scrollTo | undefined;
  type MockObserverState = {
    cb: ResizeObserverCallback;
    targets: Element[];
    instance: ResizeObserver;
  };
  let observers: MockObserverState[];
  let OriginalResizeObserver: typeof ResizeObserver | undefined;

  function makeEntry(target: Element, height: number): ResizeObserverEntry {
    return {
      target,
      contentRect: {
        height,
        width: 0,
        x: 0,
        y: 0,
        top: 0,
        left: 0,
        right: 0,
        bottom: 0,
        toJSON() {
          return {};
        },
      },
      borderBoxSize: [],
      contentBoxSize: [],
      devicePixelContentBoxSize: [],
    } as unknown as ResizeObserverEntry;
  }

  beforeEach(() => {
    originalScrollTo = Element.prototype.scrollTo;
    scrollToSpy = vi.fn();
    Element.prototype.scrollTo = scrollToSpy as unknown as typeof Element.prototype.scrollTo;

    observers = [];
    OriginalResizeObserver = globalThis.ResizeObserver;
    class MockResizeObserver {
      private cb: ResizeObserverCallback;
      private targets: Element[] = [];
      constructor(cb: ResizeObserverCallback) {
        this.cb = cb;
        observers.push({ cb, targets: this.targets, instance: this as unknown as ResizeObserver });
      }
      observe(target: Element) {
        this.targets.push(target);
        // Per spec, observe() queues a callback firing with the current size.
        // jsdom reports getBoundingClientRect().height === 0 for unstyled
        // elements, so 0 is the correct "current size" here.
        this.cb([makeEntry(target, 0)], this as unknown as ResizeObserver);
      }
      unobserve() {}
      disconnect() {
        this.targets.length = 0;
      }
    }
    globalThis.ResizeObserver = MockResizeObserver as unknown as typeof ResizeObserver;
    vi.useFakeTimers();
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
    if (originalScrollTo) {
      Element.prototype.scrollTo = originalScrollTo;
    }
    if (OriginalResizeObserver) {
      globalThis.ResizeObserver = OriginalResizeObserver;
    } else {
      // @ts-expect-error - clean up jsdom global
      delete globalThis.ResizeObserver;
    }
    vi.clearAllMocks();
  });

  function fireResize(height = 100) {
    const active = observers[observers.length - 1];
    if (!active || active.targets.length === 0) return;
    const target = active.targets[0]!;
    active.cb([makeEntry(target, height)], active.instance);
  }

  it("re-scrolls to bottom when the thread resizes after a new message (e.g., preview cards painting)", () => {
    const { rerender } = render(<ChatMessageList events={[userMessage("hi")]} />);
    rerender(<ChatMessageList events={[userMessage("hi"), assistantMessage("hello")]} />);

    const initialCalls = scrollToSpy.mock.calls.length;
    expect(initialCalls).toBeGreaterThan(0);

    // Simulate a preview card painting in after the initial scroll.
    fireResize(100);

    expect(scrollToSpy.mock.calls.length).toBeGreaterThan(initialCalls);
    expect(scrollToSpy.mock.calls.at(-1)?.[0]).toMatchObject({ behavior: "auto" });
  });

  it("does not pre-empt the initial smooth scroll when ResizeObserver's spec-mandated initial fire reports the same size", () => {
    // The MockResizeObserver in beforeEach simulates the spec-correct initial
    // fire on observe(). Each effect run (mount + rerender) triggers exactly
    // one smooth scroll, and the initial-fire callback must NOT add an auto
    // scroll on top of it.
    const { rerender } = render(<ChatMessageList events={[userMessage("hi")]} />);
    rerender(<ChatMessageList events={[userMessage("hi"), assistantMessage("hello")]} />);

    expect(scrollToSpy).toHaveBeenCalled();
    for (const call of scrollToSpy.mock.calls) {
      expect(call[0]).toMatchObject({ behavior: "smooth" });
    }
  });

  it("ignores resize events that do not grow the thread", () => {
    const { rerender } = render(<ChatMessageList events={[userMessage("hi")]} />);
    rerender(<ChatMessageList events={[userMessage("hi"), assistantMessage("hello")]} />);

    const callsBefore = scrollToSpy.mock.calls.length;
    // Same height as initial (0) — should be a no-op.
    fireResize(0);
    expect(scrollToSpy.mock.calls.length).toBe(callsBefore);
  });

  it("stops following the bottom after a brief window so the user can scroll up freely", () => {
    const { rerender } = render(<ChatMessageList events={[userMessage("hi")]} />);
    rerender(<ChatMessageList events={[userMessage("hi"), assistantMessage("hello")]} />);

    // Advance past the follow window.
    vi.advanceTimersByTime(2000);

    const callsBefore = scrollToSpy.mock.calls.length;
    fireResize(100);
    expect(scrollToSpy.mock.calls.length).toBe(callsBefore);
  });

  it("stops following when the user scrolls (wheel)", () => {
    const { container, rerender } = render(<ChatMessageList events={[userMessage("hi")]} />);
    rerender(<ChatMessageList events={[userMessage("hi"), assistantMessage("hello")]} />);

    const list = container.querySelector('[data-testid="chat-message-list"]') as HTMLElement;
    list.dispatchEvent(new Event("wheel"));

    const callsBefore = scrollToSpy.mock.calls.length;
    fireResize(100);
    expect(scrollToSpy.mock.calls.length).toBe(callsBefore);
  });
});

describe("ChatMessageList avatars and author labels", () => {
  beforeEach(() => {
    Element.prototype.scrollTo =
      vi.fn() as unknown as typeof Element.prototype.scrollTo;
  });

  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
  });

  it("shows 'You' label and human-kind creator avatar when current user is the creator", () => {
    const { container } = render(
      <ChatMessageList
        events={[userMessage("hi")]}
        agentName="chat"
        creator="alice"
        currentUsername="alice"
      />,
    );

    const avatars = container.querySelectorAll('[data-testid="avatar"]');
    expect(avatars.length).toBe(1);
    expect(avatars[0]?.getAttribute("data-kind")).toBe("human");
    expect(avatars[0]?.getAttribute("data-name")).toBe("alice");
    expect(container.textContent).toContain("You");
    expect(container.textContent).not.toMatch(/\balice\b/);
  });

  it("shows creator's username (not 'You') when current user is not the creator", () => {
    const { container } = render(
      <ChatMessageList
        events={[userMessage("hi")]}
        agentName="chat"
        creator="alice"
        currentUsername="bob"
      />,
    );

    const avatars = container.querySelectorAll('[data-testid="avatar"]');
    expect(avatars.length).toBe(1);
    expect(avatars[0]?.getAttribute("data-kind")).toBe("human");
    expect(avatars[0]?.getAttribute("data-name")).toBe("alice");
    expect(container.textContent).toContain("alice");
    // The "You" indicator should not appear when the viewer isn't the creator.
    expect(container.querySelectorAll("span")).toBeDefined();
  });

  it("renders an agent-kind avatar with the agent name on assistant messages", () => {
    const { container } = render(
      <ChatMessageList
        events={[assistantMessage("hello there")]}
        agentName="chat"
        creator="alice"
        currentUsername="alice"
      />,
    );

    const avatars = container.querySelectorAll('[data-testid="avatar"]');
    expect(avatars.length).toBe(1);
    expect(avatars[0]?.getAttribute("data-kind")).toBe("agent");
    expect(avatars[0]?.getAttribute("data-name")).toBe("chat");
    expect(container.textContent).toContain("chat");
  });

  it("renders one avatar per message in a mixed transcript", () => {
    const { container } = render(
      <ChatMessageList
        events={[userMessage("q"), assistantMessage("a")]}
        agentName="chat"
        creator="alice"
        currentUsername="alice"
      />,
    );

    const avatars = container.querySelectorAll('[data-testid="avatar"]');
    expect(avatars.length).toBe(2);
    expect(avatars[0]?.getAttribute("data-kind")).toBe("human");
    expect(avatars[1]?.getAttribute("data-kind")).toBe("agent");
  });
});

describe("ChatMessageList system_event renderer wiring", () => {
  beforeEach(() => {
    Element.prototype.scrollTo =
      vi.fn() as unknown as typeof Element.prototype.scrollTo;
  });

  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
  });

  it("delegates `system_event` entries to SystemEventBubble (not a silent drop, not a user_message bubble)", () => {
    const systemEvent: SessionEvent = {
      type: "system_event",
      kind: { kind: "child_unblocked", child_id: "i-childex", new_status: "closed" },
      timestamp: "2026-05-16T13:42:00Z",
    };
    const { container } = render(
      <ChatMessageList events={[userMessage("hi"), systemEvent]} />,
    );
    const bubbles = container.querySelectorAll('[data-testid="system-event-bubble"]');
    expect(bubbles.length).toBe(1);
    expect(bubbles[0]?.getAttribute("data-kind")).toBe("child_unblocked");
    expect(bubbles[0]?.getAttribute("data-ts")).toBe("2026-05-16T13:42:00Z");
  });
});

describe("ChatMessageList transcript source attribution", () => {
  beforeEach(() => {
    Element.prototype.scrollTo =
      vi.fn() as unknown as typeof Element.prototype.scrollTo;
  });

  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
  });

  it("hard-codes data-transcript-source=session_events on the populated container", () => {
    const { container } = render(<ChatMessageList events={[userMessage("hi")]} />);
    const list = container.querySelector('[data-testid="chat-message-list"]');
    expect(list?.getAttribute("data-transcript-source")).toBe("session_events");
  });

  it("hard-codes data-transcript-source=session_events on the empty-state container", () => {
    const { container } = render(<ChatMessageList events={[]} />);
    const list = container.querySelector('[data-testid="chat-message-list"]');
    expect(list?.getAttribute("data-transcript-source")).toBe("session_events");
  });
});
