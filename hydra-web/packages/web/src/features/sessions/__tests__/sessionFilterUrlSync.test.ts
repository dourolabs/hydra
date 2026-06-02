import { describe, it, expect } from "vitest";
import {
  applyLegacyScope,
  hasAnySessionFilterParam,
  sessionFiltersFromUrl,
  sessionFiltersToUrl,
  sessionSearchToUrl,
} from "../sessionFilterUrlSync";
import type { Filter } from "../../filters";

function repr(filters: Filter[]): string {
  return filters
    .map((f) => `${f.id}:${f.op}:${f.values.join(",")}`)
    .sort()
    .join("|");
}

describe("sessionFiltersFromUrl", () => {
  it("returns no filters when no FilterBar params are present", () => {
    const params = new URLSearchParams("q=hello");
    expect(sessionFiltersFromUrl(params)).toEqual([]);
  });

  it("parses status as a multi-value CSV", () => {
    const params = new URLSearchParams("status=running,pending");
    const filters = sessionFiltersFromUrl(params);
    expect(filters).toHaveLength(1);
    expect(filters[0]).toMatchObject({
      id: "status",
      op: "in",
      values: ["running", "pending"],
    });
  });

  it("normalises a bare-username creator value into a Principal path", () => {
    const params = new URLSearchParams("creator=alice");
    const filters = sessionFiltersFromUrl(params);
    expect(filters[0]).toMatchObject({
      id: "creator",
      values: ["users/alice"],
    });
  });

  it("preserves an already-Principal-path creator value", () => {
    const params = new URLSearchParams("creator=agents/claude");
    const filters = sessionFiltersFromUrl(params);
    expect(filters[0].values).toEqual(["agents/claude"]);
  });

  it("parses relatedIssue as multi-value and relatedChat as single-value", () => {
    const params = new URLSearchParams(
      "relatedIssue=i-1,i-2&relatedChat=c-9",
    );
    const filters = sessionFiltersFromUrl(params);
    const byId = Object.fromEntries(filters.map((f) => [f.id, f.values]));
    expect(byId.relatedIssue).toEqual(["i-1", "i-2"]);
    expect(byId.relatedChat).toEqual(["c-9"]);
  });
});

describe("sessionFiltersToUrl", () => {
  it("writes filter values to params and removes the legacy scope param", () => {
    const prev = new URLSearchParams("scope=mine&existing=keep");
    const next = sessionFiltersToUrl(prev, [
      { _uid: "u1", id: "status", op: "in", values: ["running"] },
      { _uid: "u2", id: "creator", op: "in", values: ["users/alice"] },
    ]);
    expect(next.get("status")).toBe("running");
    expect(next.get("creator")).toBe("users/alice");
    expect(next.has("scope")).toBe(false);
    expect(next.get("existing")).toBe("keep");
  });

  it("clears stale filter params when the new state drops them", () => {
    const prev = new URLSearchParams("status=running&creator=alice");
    const next = sessionFiltersToUrl(prev, []);
    expect(next.has("status")).toBe(false);
    expect(next.has("creator")).toBe(false);
  });

  it("round-trips filter[] → URL → filter[] losslessly", () => {
    const original: Filter[] = [
      { _uid: "u1", id: "status", op: "in", values: ["running", "pending"] },
      { _uid: "u2", id: "creator", op: "in", values: ["users/alice"] },
      { _uid: "u3", id: "relatedIssue", op: "in", values: ["i-1", "i-2"] },
      { _uid: "u4", id: "relatedChat", op: "in", values: ["c-9"] },
      { _uid: "u5", id: "relatedPatch", op: "in", values: ["p-a", "p-b"] },
    ];
    const url = sessionFiltersToUrl(new URLSearchParams(), original);
    const parsed = sessionFiltersFromUrl(url);
    expect(repr(parsed)).toBe(repr(original));
  });
});

describe("sessionSearchToUrl", () => {
  it("writes a non-empty q and removes it when cleared", () => {
    const withQ = sessionSearchToUrl(new URLSearchParams(), "deploy");
    expect(withQ.get("q")).toBe("deploy");
    const cleared = sessionSearchToUrl(withQ, "");
    expect(cleared.has("q")).toBe(false);
  });
});

describe("applyLegacyScope", () => {
  it("maps scope=mine to a creator chip with the user's Principal path", () => {
    const out = applyLegacyScope([], "mine", "users/alice", false);
    expect(out).toHaveLength(1);
    expect(out[0]).toMatchObject({
      id: "creator",
      values: ["users/alice"],
    });
  });

  it("maps scope=all to the empty filter list (explicit 'All' view)", () => {
    expect(applyLegacyScope([], "all", "users/alice", false)).toEqual([]);
  });

  it("is a no-op when the URL already carries explicit filter params", () => {
    const existing: Filter[] = [
      { _uid: "u", id: "status", op: "in", values: ["running"] },
    ];
    const out = applyLegacyScope(existing, "mine", "users/alice", true);
    expect(out).toBe(existing);
  });

  it("is a no-op when the user is not authenticated", () => {
    expect(applyLegacyScope([], "mine", null, false)).toEqual([]);
  });
});

describe("hasAnySessionFilterParam", () => {
  it("detects each known filter param key", () => {
    expect(
      hasAnySessionFilterParam(new URLSearchParams("status=running")),
    ).toBe(true);
    expect(
      hasAnySessionFilterParam(new URLSearchParams("relatedPatch=p-1")),
    ).toBe(true);
  });

  it("returns false for q-only URLs (q is not a FilterBar param)", () => {
    expect(hasAnySessionFilterParam(new URLSearchParams("q=foo"))).toBe(false);
  });

  it("returns false for legacy-only scope URLs", () => {
    expect(hasAnySessionFilterParam(new URLSearchParams("scope=mine"))).toBe(
      false,
    );
  });
});
