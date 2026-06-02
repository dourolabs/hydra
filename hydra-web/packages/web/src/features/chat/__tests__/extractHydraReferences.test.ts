import { describe, it, expect } from "vitest";
import { extractHydraReferences } from "../extractHydraReferences";

describe("extractHydraReferences", () => {
  it("returns an empty array for empty input", () => {
    expect(extractHydraReferences("")).toEqual([]);
  });

  it("extracts a single reference", () => {
    expect(extractHydraReferences("see [[i-abcd]]")).toEqual(["i-abcd"]);
  });

  it("extracts multiple references in source order", () => {
    expect(
      extractHydraReferences("[[i-aaaa]] and [[p-bbbb]] then [[d-cccc]]"),
    ).toEqual(["i-aaaa", "p-bbbb", "d-cccc"]);
  });

  it("dedupes repeated references, preserving the first occurrence", () => {
    expect(
      extractHydraReferences("[[i-aaaa]] [[p-bbbb]] [[i-aaaa]]"),
    ).toEqual(["i-aaaa", "p-bbbb"]);
  });

  it("skips references inside fenced code blocks", () => {
    const text = "outer [[i-aaaa]]\n```\nliteral [[i-zzzz]]\n```\nafter [[p-bbbb]]";
    expect(extractHydraReferences(text)).toEqual(["i-aaaa", "p-bbbb"]);
  });

  it("skips references inside fenced code blocks with a language hint", () => {
    const text = "before [[i-aaaa]]\n```ts\nconst x = '[[p-zzzz]]'\n```\nafter";
    expect(extractHydraReferences(text)).toEqual(["i-aaaa"]);
  });

  it("skips references inside single-backtick inline code", () => {
    expect(
      extractHydraReferences("text `[[i-aaaa]]` and [[p-bbbb]]"),
    ).toEqual(["p-bbbb"]);
  });

  it("skips references inside double-backtick inline code", () => {
    expect(
      extractHydraReferences("text ``[[i-aaaa]]`` and [[p-bbbb]]"),
    ).toEqual(["p-bbbb"]);
  });

  it("handles a mix of fenced, inline, and bare references", () => {
    const text = [
      "intro [[i-real]]",
      "```",
      "[[i-fenced]]",
      "```",
      "inline `[[p-inline]]` and bare [[p-real]]",
    ].join("\n");
    expect(extractHydraReferences(text)).toEqual(["i-real", "p-real"]);
  });

  it("ignores agent-memory-style kebab-case slugs in `[[...]]`", () => {
    expect(
      extractHydraReferences("see [[round-2-acceptance-check]] and [[i-real]]"),
    ).toEqual(["i-real"]);
  });

  it("ignores label references (`l-...`) since labels don't get cards", () => {
    expect(
      extractHydraReferences("[[l-aaaa]] but keep [[i-aaaa]]"),
    ).toEqual(["i-aaaa"]);
  });

  it("ignores unsupported prefixes outside the i/p/d/s/c set", () => {
    // Capital-letter prefixes, digits, multi-letter prefixes — none match.
    expect(
      extractHydraReferences("[[I-aaaa]] [[xy-aaaa]] [[7-aaaa]] [[i-aaaa]]"),
    ).toEqual(["i-aaaa"]);
  });

  it("returns each supported kind once across a mixed message", () => {
    const text = "[[i-issuepfx]] [[p-patchpfx]] [[d-docupfx]] [[s-sesspfx]] [[c-convpfx]]";
    expect(extractHydraReferences(text)).toEqual([
      "i-issuepfx",
      "p-patchpfx",
      "d-docupfx",
      "s-sesspfx",
      "c-convpfx",
    ]);
  });
});
