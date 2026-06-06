import { describe, it, expect } from "vitest";
import type { ProjectRecord } from "@hydra/api";
import {
  filtersFromUrl,
  filtersToUrl,
  resolveProjectKeyFilter,
} from "../filterUrlSync";
import type { Filter } from "../../filters";

function makeProject(id: string, key: string, name: string): ProjectRecord {
  return {
    project_id: id,
    version: 1,
    project: {
      key,
      name,
      statuses: [],
      default_status_key: "open",
      creator: "alice",
    },
  } as ProjectRecord;
}

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

describe("resolveProjectKeyFilter", () => {
  const projects = [makeProject("j-hidryk", "engineering-v2", "Engineering v2")];

  function projectFilter(value: string): Filter {
    return { _uid: "url:project", id: "project", op: "in", values: [value] };
  }

  it("returns `unchanged` when there is no project filter", () => {
    const result = resolveProjectKeyFilter([], projects);
    expect(result).toEqual({ outcome: "unchanged", filters: [] });
  });

  it("returns `unchanged` for a `j-`-prefixed project value (no double resolution)", () => {
    const filters = [projectFilter("j-hidryk")];
    const result = resolveProjectKeyFilter(filters, projects);
    expect(result.outcome).toBe("unchanged");
    expect(result.filters).toBe(filters);
  });

  it("returns `pending` when projects haven't loaded yet", () => {
    const filters = [projectFilter("engineering-v2")];
    const result = resolveProjectKeyFilter(filters, undefined);
    expect(result.outcome).toBe("pending");
    // Filters unchanged — caller gates server queries on the pending state.
    expect(result.filters).toBe(filters);
  });

  it("rewrites a project key to its canonical `j-<id>` form", () => {
    const filters = [projectFilter("engineering-v2")];
    const result = resolveProjectKeyFilter(filters, projects);
    expect(result.outcome).toBe("resolved");
    expect(result.filters).toEqual([
      { _uid: "url:project", id: "project", op: "in", values: ["j-hidryk"] },
    ]);
  });

  it("preserves non-project filters when resolving a project key", () => {
    const filters: Filter[] = [
      { _uid: "url:status", id: "status", op: "in", values: ["inbox"] },
      projectFilter("engineering-v2"),
    ];
    const result = resolveProjectKeyFilter(filters, projects);
    expect(result.outcome).toBe("resolved");
    expect(result.filters).toEqual([
      { _uid: "url:status", id: "status", op: "in", values: ["inbox"] },
      { _uid: "url:project", id: "project", op: "in", values: ["j-hidryk"] },
    ]);
  });

  it("drops the project filter and reports the missing key when unknown", () => {
    const filters: Filter[] = [
      { _uid: "url:status", id: "status", op: "in", values: ["inbox"] },
      projectFilter("does-not-exist"),
    ];
    const result = resolveProjectKeyFilter(filters, projects);
    expect(result.outcome).toBe("missing");
    expect(result.filters).toEqual([
      { _uid: "url:status", id: "status", op: "in", values: ["inbox"] },
    ]);
    if (result.outcome === "missing") {
      expect(result.missingKey).toBe("does-not-exist");
    }
  });
});
