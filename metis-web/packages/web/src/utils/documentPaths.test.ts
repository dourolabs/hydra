import { describe, it, expect } from "vitest";
import { extractDocumentPaths } from "./documentPaths";

describe("extractDocumentPaths", () => {
  it("extracts a single document path", () => {
    expect(extractDocumentPaths("see /docs/design.md for details")).toEqual([
      "/docs/design.md",
    ]);
  });

  it("extracts multiple document paths", () => {
    const text = "check /docs/a.md and /docs/b.md";
    const paths = extractDocumentPaths(text);
    expect(paths).toContain("/docs/a.md");
    expect(paths).toContain("/docs/b.md");
    expect(paths).toHaveLength(2);
  });

  it("deduplicates repeated paths", () => {
    const text = "/docs/a.md and /docs/a.md again";
    expect(extractDocumentPaths(text)).toEqual(["/docs/a.md"]);
  });

  it("extracts path at start of line", () => {
    expect(extractDocumentPaths("/docs/start.md")).toEqual(["/docs/start.md"]);
  });

  it("returns empty array for text with no paths", () => {
    expect(extractDocumentPaths("no document paths here")).toEqual([]);
  });

  it("does not match paths without .md extension", () => {
    expect(extractDocumentPaths("see /docs/design.txt")).toEqual([]);
  });
});
