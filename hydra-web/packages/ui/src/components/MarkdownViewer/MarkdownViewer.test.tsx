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

describe("MarkdownViewer Hydra ID linking", () => {
  function Linker({ id, raw }: { id: string; raw: string }) {
    return (
      <a href={`/${id}`} data-testid="hydra-link" data-raw={raw}>
        Title for {id}
      </a>
    );
  }

  it("renders a [[i-abc123]] token through the supplied hydraLinkComponent", () => {
    const { container } = render(
      <MarkdownViewer
        content="see [[i-abcdef]] for details"
        hydraLinkComponent={Linker}
      />,
    );
    const link = container.querySelector(
      '[data-testid="hydra-link"]',
    ) as HTMLAnchorElement | null;
    expect(link).not.toBeNull();
    expect(link!.getAttribute("href")).toBe("/i-abcdef");
    expect(link!.getAttribute("data-raw")).toBe("[[i-abcdef]]");
    expect(container.textContent).toContain("see ");
    expect(container.textContent).toContain(" for details");
    expect(container.textContent).toContain("Title for i-abcdef");
  });

  it("renders literal [[id]] text when no hydraLinkComponent is supplied", () => {
    const { container } = render(
      <MarkdownViewer content="see [[i-abcdef]] for details" />,
    );
    expect(container.querySelector('[data-testid="hydra-link"]')).toBeNull();
    expect(container.textContent).toBe("see [[i-abcdef]] for details");
  });

  it("leaves [[id]] inside fenced code blocks as literal text", () => {
    const { container } = render(
      <MarkdownViewer
        content={"```\n[[i-abcdef]]\n```"}
        hydraLinkComponent={Linker}
      />,
    );
    expect(container.querySelector('[data-testid="hydra-link"]')).toBeNull();
    expect(container.querySelector("code")?.textContent).toContain(
      "[[i-abcdef]]",
    );
  });

  it("leaves [[id]] inside inline code as literal text", () => {
    const { container } = render(
      <MarkdownViewer
        content="run `[[i-abcdef]]` carefully"
        hydraLinkComponent={Linker}
      />,
    );
    expect(container.querySelector('[data-testid="hydra-link"]')).toBeNull();
    expect(container.querySelector("code")?.textContent).toBe("[[i-abcdef]]");
  });

  it("renders malformed [[xxx]] tokens as literal text", () => {
    // Wrong prefix
    const { container: c1 } = render(
      <MarkdownViewer content="see [[x-abcdef]]" hydraLinkComponent={Linker} />,
    );
    expect(c1.querySelector('[data-testid="hydra-link"]')).toBeNull();
    expect(c1.textContent).toBe("see [[x-abcdef]]");

    // Suffix too short (3 chars, min is 4)
    const { container: c2 } = render(
      <MarkdownViewer content="see [[i-abc]]" hydraLinkComponent={Linker} />,
    );
    expect(c2.querySelector('[data-testid="hydra-link"]')).toBeNull();
    expect(c2.textContent).toBe("see [[i-abc]]");

    // Kebab-case memory slug (not a Hydra id)
    const { container: c3 } = render(
      <MarkdownViewer
        content="see [[round-2-acceptance-check]]"
        hydraLinkComponent={Linker}
      />,
    );
    expect(c3.querySelector('[data-testid="hydra-link"]')).toBeNull();
    expect(c3.textContent).toBe("see [[round-2-acceptance-check]]");
  });

  it("renders multiple tokens in the same paragraph", () => {
    const { container } = render(
      <MarkdownViewer
        content="links: [[i-abcdef]] and [[p-foobar]]"
        hydraLinkComponent={Linker}
      />,
    );
    const links = container.querySelectorAll('[data-testid="hydra-link"]');
    expect(links).toHaveLength(2);
    expect(links[0].getAttribute("href")).toBe("/i-abcdef");
    expect(links[1].getAttribute("href")).toBe("/p-foobar");
  });

  it("supports every registered prefix (i,p,d,c,s,l)", () => {
    const ids = ["i-aaaa", "p-aaaa", "d-aaaa", "c-aaaa", "s-aaaa", "l-aaaa"];
    const content = ids.map((id) => `[[${id}]]`).join(" ");
    const { container } = render(
      <MarkdownViewer content={content} hydraLinkComponent={Linker} />,
    );
    const links = container.querySelectorAll('[data-testid="hydra-link"]');
    expect(links).toHaveLength(ids.length);
    ids.forEach((id, i) => {
      expect(links[i].getAttribute("href")).toBe(`/${id}`);
    });
  });
});
