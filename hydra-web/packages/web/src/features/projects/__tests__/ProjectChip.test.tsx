// @vitest-environment jsdom
import { describe, it, expect, afterEach, vi } from "vitest";
import { cleanup, render } from "@testing-library/react";

// Resolve CSS Module class names to their kebab key so DOM assertions can
// look them up without parsing the real stylesheet.
vi.mock("../ProjectChip.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const { ProjectChip } = await import("../ProjectChip");

afterEach(() => {
  cleanup();
});

describe("ProjectChip", () => {
  it("renders the projectKey inside the mono key chip", () => {
    const { getByTestId } = render(
      <ProjectChip projectKey="engineering-v2" data-testid="chip" />,
    );
    const chip = getByTestId("chip");
    const key = chip.querySelector(".key");
    expect(key).not.toBeNull();
    expect(key!.textContent).toBe("engineering-v2");
  });

  it("does not render the name span when name is omitted", () => {
    const { getByTestId } = render(
      <ProjectChip projectKey="proj-a" data-testid="chip" />,
    );
    const chip = getByTestId("chip");
    expect(chip.querySelector(".name")).toBeNull();
  });

  it("does not render the name span when name is null", () => {
    const { getByTestId } = render(
      <ProjectChip projectKey="proj-a" name={null} data-testid="chip" />,
    );
    const chip = getByTestId("chip");
    expect(chip.querySelector(".name")).toBeNull();
  });

  it("renders the name span when name is provided", () => {
    const { getByTestId } = render(
      <ProjectChip
        projectKey="proj-a"
        name="Project Alpha"
        data-testid="chip"
      />,
    );
    const chip = getByTestId("chip");
    const name = chip.querySelector(".name");
    expect(name).not.toBeNull();
    expect(name!.textContent).toBe("Project Alpha");
  });

  it("does not force uppercase on the key", () => {
    const { getByTestId } = render(
      <ProjectChip projectKey="MixedCaseKey" data-testid="chip" />,
    );
    expect(getByTestId("chip").querySelector(".key")!.textContent).toBe(
      "MixedCaseKey",
    );
  });

  it("passes through className", () => {
    const { getByTestId } = render(
      <ProjectChip
        projectKey="x"
        className="extra-class"
        data-testid="chip"
      />,
    );
    expect(getByTestId("chip").className).toContain("extra-class");
  });
});
