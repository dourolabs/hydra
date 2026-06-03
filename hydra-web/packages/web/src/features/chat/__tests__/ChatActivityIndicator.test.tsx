import { describe, it, expect, vi, afterEach } from "vitest";
import { render, cleanup, screen } from "@testing-library/react";
import type { ActivityStep } from "../deriveActivitySteps";

vi.mock("@hydra/ui", () => ({
  Spinner: () => <span data-testid="spinner" />,
}));

vi.mock("../ChatActivityIndicator.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const { ChatActivityIndicator } = await import("../ChatActivityIndicator");

function step(overrides: Partial<ActivityStep>): ActivityStep {
  return {
    category: "think",
    verb: "Thinking…",
    detail: null,
    toolName: null,
    startTs: 0,
    endTs: null,
    ...overrides,
  };
}

afterEach(() => {
  cleanup();
});

describe("ChatActivityIndicator", () => {
  it("renders the test ids and a spinner", () => {
    render(<ChatActivityIndicator current={step({ verb: "Thinking…" })} />);
    expect(screen.getByTestId("chat-activity-indicator")).toBeTruthy();
    expect(screen.getByTestId("chat-activity-indicator-text").textContent).toBe(
      "Thinking…",
    );
    expect(screen.getByTestId("spinner")).toBeTruthy();
  });

  it("renders a known tool label as plain text", () => {
    render(
      <ChatActivityIndicator
        current={step({ category: "run", verb: "Running command" })}
      />,
    );
    expect(screen.getByTestId("chat-activity-indicator-text").textContent).toBe(
      "Running command",
    );
    // No inline-code element for known-tool branches.
    expect(
      screen.getByTestId("chat-activity-indicator-text").querySelector("code"),
    ).toBeNull();
  });

  it("renders the fallback `Using <tool_name>` with the tool name wrapped in <code>", () => {
    render(
      <ChatActivityIndicator
        current={step({
          category: "run",
          verb: "Using",
          toolName: "MysteryTool",
        })}
      />,
    );
    const text = screen.getByTestId("chat-activity-indicator-text");
    expect(text.textContent).toBe("UsingMysteryTool");
    const code = text.querySelector("code");
    expect(code?.textContent).toBe("MysteryTool");
  });

  it("prefers `detail` over `verb` when both are set", () => {
    render(
      <ChatActivityIndicator
        current={step({
          category: "run",
          verb: "Running command",
          detail: "Run the test suite",
        })}
      />,
    );
    expect(screen.getByTestId("chat-activity-indicator-text").textContent).toBe(
      "Run the test suite",
    );
  });

  it("declares an aria-live region so screen readers announce changes", () => {
    render(<ChatActivityIndicator current={step({ verb: "Thinking…" })} />);
    const root = screen.getByTestId("chat-activity-indicator");
    expect(root.getAttribute("aria-live")).toBe("polite");
    expect(root.getAttribute("role")).toBe("status");
  });
});
