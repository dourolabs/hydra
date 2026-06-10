import { describe, it, expect, vi } from "vitest";
import { render } from "@testing-library/react";
import { FlowPill } from "./FlowPill";

vi.mock("./FlowPill.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

describe("FlowPill", () => {
  it("renders the count as `num / den`", () => {
    const { container } = render(<FlowPill phase="progress" num={2} den={5} />);
    const pill = container.querySelector('[data-phase="progress"]');
    expect(pill).not.toBeNull();
    expect(pill!.textContent).toBe("2/5");
  });

  it("sets data-phase to the phase prop", () => {
    const { container } = render(<FlowPill phase="blocked" num={1} den={3} />);
    expect(container.querySelector('[data-phase="blocked"]')).not.toBeNull();
  });

  it("renders a circle SVG track", () => {
    const { container } = render(<FlowPill phase="done" num={4} den={4} />);
    expect(container.querySelector("svg circle")).not.toBeNull();
  });

  it("forwards a title attribute when provided", () => {
    const { container } = render(
      <FlowPill phase="progress" num={1} den={2} title="1 of 2 children done" />,
    );
    const pill = container.querySelector('[data-phase="progress"]');
    expect(pill?.getAttribute("title")).toBe("1 of 2 children done");
  });

  it("colours the numerator with `num` styling for blocked phase", () => {
    const { container } = render(<FlowPill phase="blocked" num={2} den={3} />);
    const numerator = container.querySelector(".num");
    expect(numerator?.textContent).toBe("2");
  });

  it("colours the numerator with `done` styling for progress/done phases", () => {
    const { container } = render(<FlowPill phase="done" num={3} den={3} />);
    const numerator = container.querySelector(".done");
    expect(numerator?.textContent).toBe("3");
  });
});
