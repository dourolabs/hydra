import { describe, it, expect, vi } from "vitest";
import { render } from "@testing-library/react";

vi.mock("./MarkdownViewer.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const { MarkdownViewer } = await import("./MarkdownViewer");
const { escapeBareOrderedListMarkers } = await import(
  "./escapeBareOrderedListMarkers"
);

describe("escapeBareOrderedListMarkers", () => {
  it("escapes a bare 'N.' line so it does not become an empty list", () => {
    expect(escapeBareOrderedListMarkers("3.")).toBe("3\\.");
    expect(escapeBareOrderedListMarkers("3. ")).toBe("3\\. ");
    expect(escapeBareOrderedListMarkers("3)")).toBe("3\\)");
  });

  it("escapes bare markers with up to 3 spaces of indent", () => {
    expect(escapeBareOrderedListMarkers("  3.")).toBe("  3\\.");
    expect(escapeBareOrderedListMarkers("   12.")).toBe("   12\\.");
  });

  it("does not escape a marker that has content after it", () => {
    expect(escapeBareOrderedListMarkers("3. do the thing")).toBe(
      "3. do the thing",
    );
    expect(escapeBareOrderedListMarkers("1) first")).toBe("1) first");
  });

  it("does not escape inline 'N.' that is not at the start of a line", () => {
    expect(escapeBareOrderedListMarkers("see step 3.")).toBe("see step 3.");
  });

  it("escapes a bare marker line surrounded by paragraphs", () => {
    expect(escapeBareOrderedListMarkers("Hello.\n\n3.\n\nGoodbye.")).toBe(
      "Hello.\n\n3\\.\n\nGoodbye.",
    );
  });

  it("leaves bare markers inside fenced code blocks untouched", () => {
    expect(escapeBareOrderedListMarkers("```\n3.\n```")).toBe("```\n3.\n```");
    expect(escapeBareOrderedListMarkers("~~~\n3.\n~~~")).toBe("~~~\n3.\n~~~");
  });

  it("leaves the empty string and content without markers untouched", () => {
    expect(escapeBareOrderedListMarkers("")).toBe("");
    expect(escapeBareOrderedListMarkers("plain text")).toBe("plain text");
  });
});

describe("MarkdownViewer", () => {
  it("renders bare '3.' as plain text, not as an empty <li>", () => {
    const { container } = render(<MarkdownViewer content="3." />);
    expect(container.querySelector("ol")).toBeNull();
    expect(container.querySelector("li")).toBeNull();
    expect(container.textContent).toBe("3.");
  });

  it("still renders '3. do the thing' as an ordered list item", () => {
    const { container } = render(
      <MarkdownViewer content="3. do the thing" />,
    );
    const ol = container.querySelector("ol");
    expect(ol).not.toBeNull();
    const items = container.querySelectorAll("li");
    expect(items).toHaveLength(1);
    expect(items[0].textContent).toBe("do the thing");
  });

  it("renders inline 'see step 3.' as plain text", () => {
    const { container } = render(<MarkdownViewer content="see step 3." />);
    expect(container.querySelector("ol")).toBeNull();
    expect(container.querySelector("li")).toBeNull();
    expect(container.textContent).toBe("see step 3.");
  });
});
