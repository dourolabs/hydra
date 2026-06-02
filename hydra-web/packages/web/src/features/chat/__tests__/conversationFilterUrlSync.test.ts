import { describe, it, expect } from "vitest";
import {
  filtersFromUrl,
  filtersToUrl,
  searchToUrl,
  legacyScopeRedirect,
  defaultCreatorFilter,
  SEARCH_URL_PARAM,
} from "../conversationFilterUrlSync";
import type { Filter } from "../../filters";

describe("filtersFromUrl", () => {
  it("returns no filters when the URL has no relevant params", () => {
    expect(filtersFromUrl(new URLSearchParams(""))).toEqual([]);
  });

  it("parses ?status= into a single-select status filter", () => {
    const filters = filtersFromUrl(new URLSearchParams("status=active"));
    expect(filters).toEqual([
      { _uid: "url:status", id: "status", op: "in", values: ["active"] },
    ]);
  });

  it("parses ?creator= as a Principal path, normalising bare usernames", () => {
    const fromBare = filtersFromUrl(new URLSearchParams("creator=alice"));
    expect(fromBare).toEqual([
      {
        _uid: "url:creator",
        id: "creator",
        op: "in",
        values: ["users/alice"],
      },
    ]);

    const fromPath = filtersFromUrl(new URLSearchParams("creator=agents/swe"));
    expect(fromPath).toEqual([
      {
        _uid: "url:creator",
        id: "creator",
        op: "in",
        values: ["agents/swe"],
      },
    ]);
  });

  it("ignores unrelated query params", () => {
    const filters = filtersFromUrl(
      new URLSearchParams("status=closed&unrelated=x&q=hello"),
    );
    expect(filters).toEqual([
      { _uid: "url:status", id: "status", op: "in", values: ["closed"] },
    ]);
  });
});

describe("filtersToUrl", () => {
  it("writes status and creator chips to the URL", () => {
    const filters: Filter[] = [
      { _uid: "f1", id: "status", op: "in", values: ["active"] },
      { _uid: "f2", id: "creator", op: "in", values: ["users/alice"] },
    ];
    const next = filtersToUrl(new URLSearchParams(""), filters);
    expect(next.get("status")).toBe("active");
    expect(next.get("creator")).toBe("users/alice");
  });

  it("strips legacy ?scope= when writing FilterBar state", () => {
    const next = filtersToUrl(new URLSearchParams("scope=mine"), [
      { _uid: "f1", id: "creator", op: "in", values: ["users/alice"] },
    ]);
    expect(next.has("scope")).toBe(false);
    expect(next.get("creator")).toBe("users/alice");
  });

  it("clears params when filters are emptied (Clear All / remove chip)", () => {
    const next = filtersToUrl(
      new URLSearchParams("status=active&creator=alice&q=hello"),
      [],
    );
    expect(next.has("status")).toBe(false);
    expect(next.has("creator")).toBe(false);
    // Free-text search lives on a separate axis; filtersToUrl shouldn't touch it.
    expect(next.get(SEARCH_URL_PARAM)).toBe("hello");
  });

  it("skips filters with empty value lists (in-flight UI state)", () => {
    const next = filtersToUrl(new URLSearchParams(""), [
      { _uid: "f1", id: "status", op: "in", values: [] },
    ]);
    expect(next.has("status")).toBe(false);
  });

  it("ignores filter ids that don't map to a URL param", () => {
    const next = filtersToUrl(new URLSearchParams(""), [
      { _uid: "f1", id: "relatedIssue", op: "in", values: ["i-aaa"] },
    ]);
    expect(next.toString()).toBe("");
  });

  it("round-trips status and creator chips through URL", () => {
    const filters: Filter[] = [
      { _uid: "f1", id: "status", op: "in", values: ["idle"] },
      { _uid: "f2", id: "creator", op: "in", values: ["users/alice"] },
    ];
    const url = filtersToUrl(new URLSearchParams(""), filters);
    const roundTripped = filtersFromUrl(url);
    expect(roundTripped).toEqual([
      { _uid: "url:status", id: "status", op: "in", values: ["idle"] },
      { _uid: "url:creator", id: "creator", op: "in", values: ["users/alice"] },
    ]);
  });
});

describe("searchToUrl", () => {
  it("sets ?q= when given a non-empty value", () => {
    const next = searchToUrl(new URLSearchParams(""), "hello");
    expect(next.get(SEARCH_URL_PARAM)).toBe("hello");
  });

  it("strips ?q= when given an empty value", () => {
    const next = searchToUrl(new URLSearchParams("q=stale"), "");
    expect(next.has(SEARCH_URL_PARAM)).toBe(false);
  });
});

describe("legacyScopeRedirect", () => {
  it("returns null when ?scope= is not present", () => {
    expect(legacyScopeRedirect(new URLSearchParams(""), "alice")).toBeNull();
    expect(legacyScopeRedirect(new URLSearchParams("q=x"), "alice")).toBeNull();
  });

  it("resolves ?scope=mine into a creator chip + URL rewrite", () => {
    const out = legacyScopeRedirect(new URLSearchParams("scope=mine"), "alice");
    expect(out).not.toBeNull();
    expect(out!.filters).toEqual([
      {
        _uid: "url:creator",
        id: "creator",
        op: "in",
        values: ["users/alice"],
      },
    ]);
    expect(out!.nextParams.get("creator")).toBe("users/alice");
    expect(out!.nextParams.has("scope")).toBe(false);
  });

  it("resolves ?scope=all into no filter + URL rewrite", () => {
    const out = legacyScopeRedirect(new URLSearchParams("scope=all"), "alice");
    expect(out).not.toBeNull();
    expect(out!.filters).toEqual([]);
    expect(out!.nextParams.has("scope")).toBe(false);
    expect(out!.nextParams.has("creator")).toBe(false);
  });

  it("preserves explicit FilterBar params over the legacy scope", () => {
    const out = legacyScopeRedirect(
      new URLSearchParams("scope=mine&status=closed"),
      "alice",
    );
    expect(out).not.toBeNull();
    expect(out!.nextParams.has("scope")).toBe(false);
    expect(out!.nextParams.get("status")).toBe("closed");
    expect(out!.filters).toEqual([
      { _uid: "url:status", id: "status", op: "in", values: ["closed"] },
    ]);
  });

  it("no-ops scope=mine when no current user is known", () => {
    const out = legacyScopeRedirect(new URLSearchParams("scope=mine"), null);
    expect(out).not.toBeNull();
    expect(out!.filters).toEqual([]);
    expect(out!.nextParams.has("creator")).toBe(false);
    expect(out!.nextParams.has("scope")).toBe(false);
  });
});

describe("defaultCreatorFilter", () => {
  it("returns a single creator chip for the current user", () => {
    expect(defaultCreatorFilter("alice")).toEqual([
      {
        _uid: "url:creator",
        id: "creator",
        op: "in",
        values: ["users/alice"],
      },
    ]);
  });

  it("returns no filters when no user is known", () => {
    expect(defaultCreatorFilter(null)).toEqual([]);
  });
});
