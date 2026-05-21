import { describe, it, expect, vi } from "vitest";
import { render } from "@testing-library/react";
import { TypeChip, issueTypeDisplayLabel } from "./TypeChip";

vi.mock("./TypeChip.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

describe("issueTypeDisplayLabel", () => {
  it("maps review-request to review", () => {
    expect(issueTypeDisplayLabel("review-request")).toBe("review");
  });

  it("maps merge-request to merge", () => {
    expect(issueTypeDisplayLabel("merge-request")).toBe("merge");
  });

  it("passes other types through unchanged", () => {
    expect(issueTypeDisplayLabel("task")).toBe("task");
    expect(issueTypeDisplayLabel("bug")).toBe("bug");
    expect(issueTypeDisplayLabel("feature")).toBe("feature");
    expect(issueTypeDisplayLabel("chore")).toBe("chore");
    expect(issueTypeDisplayLabel("unknown")).toBe("unknown");
  });
});

describe("TypeChip", () => {
  it("renders 'review' text but keeps data-type='review-request'", () => {
    const { container } = render(<TypeChip type="review-request" />);
    const span = container.querySelector("span");
    expect(span).not.toBeNull();
    expect(span?.getAttribute("data-type")).toBe("review-request");
    expect(span?.textContent).toBe("review");
  });

  it("renders 'merge' text but keeps data-type='merge-request'", () => {
    const { container } = render(<TypeChip type="merge-request" />);
    const span = container.querySelector("span");
    expect(span).not.toBeNull();
    expect(span?.getAttribute("data-type")).toBe("merge-request");
    expect(span?.textContent).toBe("merge");
  });

  it("renders other types unchanged", () => {
    const { container } = render(<TypeChip type="task" />);
    const span = container.querySelector("span");
    expect(span?.getAttribute("data-type")).toBe("task");
    expect(span?.textContent).toBe("task");
  });
});
