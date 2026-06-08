import { describe, it, expect } from "vitest";
import type { ProjectRecord } from "@hydra/api";
import {
  filtersFromUrl,
  filtersToUrl,
  resolveProjectFromUrl,
  PROJECT_KEY_URL_PARAM,
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
      priority: 0,
    },
  } as ProjectRecord;
}

describe("filterUrlSync", () => {
  it("rehydrates both project and status chips from ?project=...&status=...", () => {
    const filters = filtersFromUrl(
      new URLSearchParams("project=j-engv2&status=inbox"),
    );

    expect(filters).toHaveLength(2);
    expect(filters.find((f) => f.id === "project")).toMatchObject({
      id: "project",
      op: "in",
      values: ["j-engv2"],
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

  it("strips ?project_key= on URL rewrite (transient resolution input)", () => {
    // `?project_key=` is the resolution input; once filter state owns the
    // resolved id, filtersToUrl rewrites to canonical `?project=j-<id>` and
    // drops the slug param so the URL doesn't carry both forms.
    const prev = new URLSearchParams("project_key=engineering-v2");
    const next = filtersToUrl(prev, [
      { _uid: "url:project", id: "project", op: "in", values: ["j-engv2"] },
    ]);
    expect(next.get("project_key")).toBeNull();
    expect(next.get("project")).toBe("j-engv2");
  });
});

describe("resolveProjectFromUrl", () => {
  // The two URL params share the project-selection job, with disjoint value
  // spaces enforced at parse time — see the docstring on
  // `resolveProjectFromUrl` and `docs/architecture/api-wire-contract.md`
  // ("Parameter forms must be mutually exclusive by construction").

  const projects = [makeProject("j-hidryk", "engineering-v2", "Engineering v2")];

  function projectFilter(value: string): Filter {
    return { _uid: "url:project", id: "project", op: "in", values: [value] };
  }

  function params(search: string): URLSearchParams {
    return new URLSearchParams(search);
  }

  it("returns `unchanged` when there is no project filter and no ?project_key=", () => {
    const result = resolveProjectFromUrl([], params(""), projects);
    expect(result).toEqual({ outcome: "unchanged", filters: [] });
  });

  it("returns `unchanged` for a `j-`-prefixed `?project=` value", () => {
    const filters = [projectFilter("j-hidryk")];
    const result = resolveProjectFromUrl(filters, params("project=j-hidryk"), projects);
    expect(result.outcome).toBe("unchanged");
    expect(result.filters).toBe(filters);
  });

  it("returns `invalid` for a non-`j-` `?project=` value (drops the project filter)", () => {
    const filters = [projectFilter("engineering-v2")];
    const result = resolveProjectFromUrl(
      filters,
      params("project=engineering-v2"),
      projects,
    );
    expect(result.outcome).toBe("invalid");
    expect(result.filters).toEqual([]);
    if (result.outcome === "invalid") {
      expect(result.invalidValue).toBe("engineering-v2");
    }
  });

  it("returns `pending` when ?project_key= is set but projects haven't loaded", () => {
    const result = resolveProjectFromUrl(
      [],
      params(`${PROJECT_KEY_URL_PARAM}=engineering-v2`),
      undefined,
    );
    expect(result.outcome).toBe("pending");
    expect(result.filters).toEqual([]);
  });

  it("rewrites a ?project_key=<slug> to canonical filter state with `j-<id>`", () => {
    const result = resolveProjectFromUrl(
      [],
      params(`${PROJECT_KEY_URL_PARAM}=engineering-v2`),
      projects,
    );
    expect(result.outcome).toBe("resolved");
    expect(result.filters).toEqual([
      { _uid: "url:project", id: "project", op: "in", values: ["j-hidryk"] },
    ]);
  });

  it("preserves non-project filters when resolving ?project_key=", () => {
    const filters: Filter[] = [
      { _uid: "url:status", id: "status", op: "in", values: ["inbox"] },
    ];
    const result = resolveProjectFromUrl(
      filters,
      params(`status=inbox&${PROJECT_KEY_URL_PARAM}=engineering-v2`),
      projects,
    );
    expect(result.outcome).toBe("resolved");
    expect(result.filters).toEqual([
      { _uid: "url:status", id: "status", op: "in", values: ["inbox"] },
      { _uid: "url:project", id: "project", op: "in", values: ["j-hidryk"] },
    ]);
  });

  it("drops the project filter and reports the missing key for an unknown ?project_key=", () => {
    const filters: Filter[] = [
      { _uid: "url:status", id: "status", op: "in", values: ["inbox"] },
    ];
    const result = resolveProjectFromUrl(
      filters,
      params(`status=inbox&${PROJECT_KEY_URL_PARAM}=does-not-exist`),
      projects,
    );
    expect(result.outcome).toBe("missing");
    expect(result.filters).toEqual([
      { _uid: "url:status", id: "status", op: "in", values: ["inbox"] },
    ]);
    if (result.outcome === "missing") {
      expect(result.missingKey).toBe("does-not-exist");
    }
  });

  it("returns `invalid` when ?project_key= itself is `j-`-prefixed (value-space violation)", () => {
    // The two URL params are mutually exclusive by construction at parse
    // time: ?project= rejects non-`j-`, ?project_key= rejects `j-`. A
    // `j-`-prefixed token in ?project_key= can't be silently re-classified.
    const result = resolveProjectFromUrl(
      [projectFilter("j-foo")],
      params(`project=j-foo&${PROJECT_KEY_URL_PARAM}=j-hidryk`),
      projects,
    );
    expect(result.outcome).toBe("invalid");
    expect(result.filters).toEqual([]);
    if (result.outcome === "invalid") {
      expect(result.invalidValue).toBe("j-hidryk");
    }
  });

  it("?project_key= takes precedence over ?project= when both are present", () => {
    // Steady state never has both; the resolver rewrites the URL so only
    // `?project=j-<id>` remains. But if a hand-crafted URL sets both, the
    // slug-form resolution wins because that's the canonicalization flow.
    const result = resolveProjectFromUrl(
      [projectFilter("j-other")],
      params(`project=j-other&${PROJECT_KEY_URL_PARAM}=engineering-v2`),
      projects,
    );
    expect(result.outcome).toBe("resolved");
    expect(result.filters).toEqual([
      { _uid: "url:project", id: "project", op: "in", values: ["j-hidryk"] },
    ]);
  });
});
