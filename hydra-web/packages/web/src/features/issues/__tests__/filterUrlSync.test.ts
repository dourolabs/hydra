import { describe, it, expect } from "vitest";
import { filtersFromUrl, filtersToUrl } from "../filterUrlSync";

describe("filterUrlSync", () => {
  it("rehydrates both project and status chips from ?project=...&status=...", () => {
    const filters = filtersFromUrl(
      new URLSearchParams("project=engineering-v2&status=inbox"),
    );

    expect(filters).toHaveLength(2);
    expect(filters.find((f) => f.id === "project")).toMatchObject({
      id: "project",
      op: "in",
      values: ["engineering-v2"],
    });
    expect(filters.find((f) => f.id === "status")).toMatchObject({
      id: "status",
      op: "in",
      values: ["inbox"],
    });
  });

  it("round-trips a project chip back to the URL", () => {
    const next = filtersToUrl(new URLSearchParams(""), [
      { _uid: "url:project", id: "project", op: "in", values: ["j-engv2"] },
    ]);
    expect(next.get("project")).toBe("j-engv2");
  });

  it("treats project as single-select (one value, no comma split)", () => {
    const filters = filtersFromUrl(new URLSearchParams("project=j-engv2,j-other"));
    const project = filters.find((f) => f.id === "project");
    // Single-select takes the raw value as one entry, not split on commas.
    expect(project?.values).toEqual(["j-engv2,j-other"]);
  });
});
