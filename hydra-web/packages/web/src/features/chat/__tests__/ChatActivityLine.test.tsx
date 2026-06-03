import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import { render, cleanup, screen, fireEvent, act } from "@testing-library/react";
import type { ActivityRun, ActivityStep } from "../deriveActivitySteps";

// Identity proxy so className lookups round-trip the key.
vi.mock("../ChatActivityLine.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const { ChatActivityLine } = await import("../ChatActivityLine");

function step(overrides: Partial<ActivityStep> = {}): ActivityStep {
  return {
    category: "read",
    verb: "Reading file",
    detail: null,
    toolName: null,
    startTs: 0,
    endTs: null,
    ...overrides,
  };
}

function liveRun(
  current: ActivityStep,
  steps: ActivityStep[] = [current],
  startedAt = 0,
): ActivityRun {
  return { steps, current, state: "live", startedAt };
}

function doneRun(steps: ActivityStep[], startedAt = 0): ActivityRun {
  return { steps, current: null, state: "done", startedAt };
}

afterEach(() => {
  cleanup();
});

describe("ChatActivityLine — visibility gating", () => {
  it("renders nothing when current is null and there are no steps", () => {
    const { container } = render(
      <ChatActivityLine run={{ steps: [], current: null, state: "live", startedAt: 0 }} />,
    );
    expect(container.firstChild).toBeNull();
  });

  it("renders when the run terminated with at least one historical step", () => {
    const s = step({
      category: "edit",
      verb: "Editing file",
      startTs: 0,
      endTs: 2_000,
    });
    render(<ChatActivityLine run={doneRun([s])} now={() => 5_000} />);
    expect(screen.getByTestId("chat-activity-line")).toBeTruthy();
  });
});

describe("ChatActivityLine — collapsed row reflects current step", () => {
  it("shows the current step's verb, detail, and category", () => {
    const s = step({
      category: "edit",
      verb: "Editing file",
      detail: "patch.tsx",
    });
    render(<ChatActivityLine run={liveRun(s)} now={() => 1_000} />);

    const root = screen.getByTestId("chat-activity-line");
    expect(root.getAttribute("data-live")).toBe("true");
    expect(root.getAttribute("data-state")).toBe("live");
    expect(root.getAttribute("data-category")).toBe("edit");
    expect(root.getAttribute("data-open")).toBe("false");

    expect(screen.getByTestId("chat-activity-line-verb").textContent).toBe("Editing file");
    expect(screen.getByTestId("chat-activity-line-detail").textContent).toBe("patch.tsx");
  });

  it("falls back to tool-name code when verb has no detail", () => {
    const s = step({
      category: "run",
      verb: "Using",
      toolName: "MysteryTool",
    });
    render(<ChatActivityLine run={liveRun(s)} now={() => 0} />);

    expect(screen.getByTestId("chat-activity-line-tool").textContent).toBe("MysteryTool");
  });

  it("wraps the verb/detail in a polite live region so AT announces transitions", () => {
    const s = step({
      category: "edit",
      verb: "Editing file",
      detail: "patch.tsx",
    });
    render(<ChatActivityLine run={liveRun(s)} now={() => 0} />);

    const verb = screen.getByTestId("chat-activity-line-verb");
    const liveRegion = verb.parentElement;
    expect(liveRegion).not.toBeNull();
    expect(liveRegion!.getAttribute("role")).toBe("status");
    expect(liveRegion!.getAttribute("aria-live")).toBe("polite");
  });

  it("formats the run timer as M:SS using tabular-nums", () => {
    const s = step({ startTs: 0 });
    render(<ChatActivityLine run={liveRun(s, [s], 0)} now={() => 75_400} />);

    expect(screen.getByTestId("chat-activity-line-timer").textContent).toContain("1:15");
  });
});

describe("ChatActivityLine — click toggles feed", () => {
  it("renders the toggle as a button with aria-expanded", () => {
    const s = step();
    render(<ChatActivityLine run={liveRun(s)} now={() => 0} />);
    const btn = screen.getByTestId("chat-activity-line-toggle");
    expect(btn.tagName).toBe("BUTTON");
    expect(btn.getAttribute("aria-expanded")).toBe("false");
  });

  it("opens and closes the feed on click", () => {
    const s = step();
    render(<ChatActivityLine run={liveRun(s)} now={() => 0} />);

    expect(screen.queryByTestId("chat-activity-line-feed")).toBeNull();

    const btn = screen.getByTestId("chat-activity-line-toggle");
    fireEvent.click(btn);

    expect(screen.getByTestId("chat-activity-line-feed")).toBeTruthy();
    expect(btn.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByTestId("chat-activity-line").getAttribute("data-open")).toBe("true");

    fireEvent.click(btn);
    expect(screen.queryByTestId("chat-activity-line-feed")).toBeNull();
  });
});

describe("ChatActivityLine — expanded feed", () => {
  it("renders one row per step with correct data-state for closed vs active", () => {
    const closed = step({
      category: "read",
      verb: "Reading file",
      detail: "a.ts",
      startTs: 0,
      endTs: 1_500,
    });
    const active = step({
      category: "edit",
      verb: "Editing file",
      detail: "b.ts",
      startTs: 1_500,
      endTs: null,
    });
    render(<ChatActivityLine run={liveRun(active, [closed, active])} now={() => 4_500} />);

    fireEvent.click(screen.getByTestId("chat-activity-line-toggle"));

    const rows = screen.getAllByTestId("chat-activity-line-step");
    expect(rows.length).toBe(2);
    expect(rows[0].getAttribute("data-state")).toBe("done");
    expect(rows[0].getAttribute("data-category")).toBe("read");
    expect(rows[1].getAttribute("data-state")).toBe("active");
    expect(rows[1].getAttribute("data-category")).toBe("edit");
  });

  it("falls back the active row's duration to the ticking clock until endTs lands", () => {
    const active = step({
      category: "run",
      verb: "Running command",
      startTs: 0,
      endTs: null,
    });
    render(<ChatActivityLine run={liveRun(active)} now={() => 3_400} />);
    fireEvent.click(screen.getByTestId("chat-activity-line-toggle"));

    const row = screen.getByTestId("chat-activity-line-step");
    // Per-step `.dur` text is the only direct number in the row apart from
    // the verb / detail / tool tags above; assert it contains the live ticks.
    expect(row.textContent).toContain("3.4s");
  });
});

describe("ChatActivityLine — terminal state", () => {
  const closedStep = (overrides: Partial<ActivityStep> = {}) =>
    step({ category: "read", startTs: 0, endTs: 1_000, ...overrides });

  it("falls back to the done category when current is null", () => {
    const s = closedStep();
    render(<ChatActivityLine run={doneRun([s])} now={() => 999_999} />);
    const root = screen.getByTestId("chat-activity-line");
    expect(root.getAttribute("data-state")).toBe("done");
    expect(root.getAttribute("data-live")).toBe("false");
    expect(root.getAttribute("data-category")).toBe("done");
  });

  it("freezes the timer at the last step's endTs (does not tick forward)", () => {
    const s = closedStep({ endTs: 4_000 });
    const { rerender } = render(<ChatActivityLine run={doneRun([s], 0)} now={() => 999_999} />);
    expect(screen.getByTestId("chat-activity-line-timer").textContent).toContain("0:04");
    rerender(<ChatActivityLine run={doneRun([s], 0)} now={() => 8_888_888} />);
    expect(screen.getByTestId("chat-activity-line-timer").textContent).toContain("0:04");
  });
});

describe("ChatActivityLine — live timer ticks", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("re-renders the timer at the tick interval while live", () => {
    let clock = 0;
    const s = step({ startTs: 0 });
    render(<ChatActivityLine run={liveRun(s)} now={() => clock} />);

    expect(screen.getByTestId("chat-activity-line-timer").textContent).toContain("0:00");

    act(() => {
      clock = 12_000;
      vi.advanceTimersByTime(300);
    });
    expect(screen.getByTestId("chat-activity-line-timer").textContent).toContain("0:12");
  });

  it("stops the tick loop when state flips to done", () => {
    let clock = 0;
    const active = step({ category: "run", startTs: 0, endTs: null });
    const { rerender } = render(
      <ChatActivityLine run={liveRun(active, [active], 0)} now={() => clock} />,
    );
    act(() => {
      clock = 5_000;
      vi.advanceTimersByTime(300);
    });
    expect(screen.getByTestId("chat-activity-line-timer").textContent).toContain("0:05");

    const settled = step({
      category: "run",
      startTs: 0,
      endTs: 5_000,
    });
    rerender(<ChatActivityLine run={doneRun([settled], 0)} now={() => clock} />);

    // Advance well past the tick interval; timer must not move past 0:05.
    act(() => {
      clock = 999_999;
      vi.advanceTimersByTime(5_000);
    });
    expect(screen.getByTestId("chat-activity-line-timer").textContent).toContain("0:05");
  });
});
