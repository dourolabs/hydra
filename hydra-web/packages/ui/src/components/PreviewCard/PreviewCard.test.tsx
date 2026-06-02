import { describe, it, expect, vi } from "vitest";
import { fireEvent, render } from "@testing-library/react";
import { PreviewCard, type PreviewCardTone } from "./PreviewCard";

vi.mock("./PreviewCard.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const TONES: PreviewCardTone[] = [
  "open",
  "in-progress",
  "closed",
  "failed",
  "dropped",
  "blocked",
  "rejected",
  "neutral",
];

describe("PreviewCard", () => {
  it("renders as a button with the supplied aria-label", () => {
    const { getByRole } = render(
      <PreviewCard
        tone="open"
        topRow={<span>top</span>}
        title="Hello"
        ariaLabel="Open issue: Hello"
      />,
    );
    const button = getByRole("button", { name: "Open issue: Hello" });
    expect(button).toBeDefined();
    expect(button.tagName).toBe("BUTTON");
    expect((button as HTMLButtonElement).type).toBe("button");
  });

  it("fires the onClick handler when clicked", () => {
    const onClick = vi.fn();
    const { getByRole } = render(
      <PreviewCard
        tone="open"
        topRow={null}
        title="Title"
        ariaLabel="card"
        onClick={onClick}
      />,
    );
    fireEvent.click(getByRole("button"));
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("renders body excerpt and footer when provided", () => {
    const { getByText, queryByText, rerender } = render(
      <PreviewCard
        tone="open"
        topRow={null}
        title="t"
        bodyExcerpt="excerpt text"
        footer={<span>footer text</span>}
        ariaLabel="card"
      />,
    );
    expect(getByText("excerpt text")).toBeDefined();
    expect(getByText("footer text")).toBeDefined();

    // Omitting them removes both from the DOM (no empty wrapper).
    rerender(
      <PreviewCard tone="open" topRow={null} title="t" ariaLabel="card" />,
    );
    expect(queryByText("excerpt text")).toBeNull();
    expect(queryByText("footer text")).toBeNull();
  });

  it.each(TONES)("sets data-tone for tone=%s", (tone) => {
    const { getByRole } = render(
      <PreviewCard tone={tone} topRow={null} title="t" ariaLabel="card" />,
    );
    expect(getByRole("button").getAttribute("data-tone")).toBe(tone);
  });

  it("max-width is enforced via the CSS module (constants live there)", () => {
    // We can't compute layout in jsdom, but we can assert the class is applied
    // — the CSS module owns the 520px cap so per-kind callers don't repeat it.
    const { getByRole } = render(
      <PreviewCard tone="open" topRow={null} title="t" ariaLabel="card" />,
    );
    expect(getByRole("button").className).toContain("card");
  });
});
