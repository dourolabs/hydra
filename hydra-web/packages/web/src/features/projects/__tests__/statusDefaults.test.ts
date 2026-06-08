import { describe, it, expect } from "vitest";
import { slugifyStatusKey } from "../statusDefaults";

describe("slugifyStatusKey", () => {
  it("lowercases and dash-joins words", () => {
    expect(slugifyStatusKey("In Review")).toBe("in-review");
    expect(slugifyStatusKey("BLOCKED")).toBe("blocked");
  });

  it("collapses runs of non-alphanumeric characters into a single dash", () => {
    expect(slugifyStatusKey("In  Review!!")).toBe("in-review");
    expect(slugifyStatusKey("a / b / c")).toBe("a-b-c");
  });

  it("strips leading and trailing dashes", () => {
    expect(slugifyStatusKey(" leading")).toBe("leading");
    expect(slugifyStatusKey("trailing  ")).toBe("trailing");
    expect(slugifyStatusKey("--surrounded--")).toBe("surrounded");
  });

  it("returns an empty string when no alphanumeric characters are present", () => {
    expect(slugifyStatusKey("")).toBe("");
    expect(slugifyStatusKey("@@@!!!")).toBe("");
    expect(slugifyStatusKey("   ")).toBe("");
  });

  it("preserves digits and existing kebab-case", () => {
    expect(slugifyStatusKey("v1")).toBe("v1");
    expect(slugifyStatusKey("in-progress")).toBe("in-progress");
  });
});
