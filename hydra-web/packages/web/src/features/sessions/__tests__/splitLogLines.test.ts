import { describe, it, expect } from "vitest";
import { splitLogLines } from "../splitLogLines";

describe("splitLogLines", () => {
  it("splits LF-terminated input", () => {
    expect(splitLogLines("a\nb\nc")).toEqual(["a", "b", "c"]);
  });

  it("splits CRLF-terminated input without leaving a trailing CR", () => {
    expect(splitLogLines("a\r\nb\r\nc")).toEqual(["a", "b", "c"]);
  });

  it("strips bare CRs embedded inside a line", () => {
    expect(splitLogLines("Progress: 10%\rProgress: 50%\rProgress: 100%")).toEqual([
      "Progress: 10%Progress: 50%Progress: 100%",
    ]);
  });

  it("handles a mix of CRLF, LF, and embedded CR", () => {
    expect(splitLogLines("a\r\nb\nc\rd\r\ne")).toEqual(["a", "b", "cd", "e"]);
  });

  it("preserves empty lines", () => {
    expect(splitLogLines("a\n\nb")).toEqual(["a", "", "b"]);
  });

  it("returns a single empty string for an empty input", () => {
    expect(splitLogLines("")).toEqual([""]);
  });
});
