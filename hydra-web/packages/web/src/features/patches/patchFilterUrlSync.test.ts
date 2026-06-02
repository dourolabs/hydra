import { describe, it, expect } from "vitest";
import type { Filter } from "../filters";
import {
  filtersFromUrl,
  filtersToUrl,
  searchToUrl,
} from "./patchFilterUrlSync";

function params(input: string): URLSearchParams {
  return new URLSearchParams(input);
}

function mkFilter(id: string, values: string[]): Filter {
  return { _uid: `url:${id}`, id, op: "in", values };
}

describe("patchFilterUrlSync", () => {
  describe("filtersFromUrl", () => {
    it("returns no filters when the URL is empty", () => {
      expect(filtersFromUrl(params(""))).toEqual([]);
    });

    it("parses status as a multi-value (comma-separated) filter", () => {
      expect(filtersFromUrl(params("status=Open,Merged"))).toEqual([
        mkFilter("status", ["Open", "Merged"]),
      ]);
    });

    it("parses repository as a single-value filter", () => {
      // Single-select filters take the raw param verbatim — commas in repo
      // names are not split.
      expect(filtersFromUrl(params("repository=acme/web-app"))).toEqual([
        mkFilter("repository", ["acme/web-app"]),
      ]);
    });

    it("normalises bare author usernames to Principal paths", () => {
      expect(filtersFromUrl(params("author=alice"))).toEqual([
        mkFilter("author", ["users/alice"]),
      ]);
      // Already-Principal-shaped values pass through unchanged.
      expect(filtersFromUrl(params("author=agents/swe"))).toEqual([
        mkFilter("author", ["agents/swe"]),
      ]);
    });

    it("parses relation filters as multi-value", () => {
      expect(filtersFromUrl(params("relatedIssue=i-aa,i-bb"))).toEqual([
        mkFilter("relatedIssue", ["i-aa", "i-bb"]),
      ]);
      expect(filtersFromUrl(params("relatedSession=s-aa"))).toEqual([
        mkFilter("relatedSession", ["s-aa"]),
      ]);
    });

    it("ignores unknown params", () => {
      expect(filtersFromUrl(params("garbage=foo"))).toEqual([]);
    });
  });

  describe("filtersToUrl", () => {
    it("writes status as a comma-joined list", () => {
      const out = filtersToUrl(params(""), [
        mkFilter("status", ["Open", "Merged"]),
      ]);
      expect(out.get("status")).toBe("Open,Merged");
    });

    it("writes repository as a single value", () => {
      const out = filtersToUrl(params(""), [
        mkFilter("repository", ["acme/web-app"]),
      ]);
      expect(out.get("repository")).toBe("acme/web-app");
    });

    it("clears stale filter params when filters change", () => {
      // Start with two filters in the URL; remove one and confirm the URL no
      // longer carries it.
      const out = filtersToUrl(params("status=Open&author=alice"), [
        mkFilter("status", ["Open"]),
      ]);
      expect(out.get("status")).toBe("Open");
      expect(out.get("author")).toBeNull();
    });

    it("does not touch non-filter params", () => {
      const out = filtersToUrl(params("q=oauth&selected=x"), [
        mkFilter("status", ["Open"]),
      ]);
      expect(out.get("q")).toBe("oauth");
      expect(out.get("selected")).toBe("x");
    });

    it("drops empty-values filters (mid-add UI state)", () => {
      const out = filtersToUrl(params(""), [mkFilter("status", [])]);
      expect(out.toString()).toBe("");
    });
  });

  describe("round-trip", () => {
    it("preserves status / repository / author / relations through one cycle", () => {
      const input: Filter[] = [
        mkFilter("status", ["Open", "Merged"]),
        mkFilter("repository", ["acme/web-app"]),
        mkFilter("author", ["users/alice"]),
        mkFilter("relatedIssue", ["i-aa", "i-bb"]),
        mkFilter("relatedSession", ["s-aa"]),
      ];
      const written = filtersToUrl(params(""), input);
      const read = filtersFromUrl(written);
      // Round-tripping through the URL drops `_uid` (re-keyed from id) but
      // preserves id / op / values exactly.
      expect(read).toEqual(input);
    });
  });

  describe("searchToUrl", () => {
    it("sets ?q= when non-empty", () => {
      const out = searchToUrl(params("status=Open"), "oauth");
      expect(out.get("q")).toBe("oauth");
      expect(out.get("status")).toBe("Open");
    });

    it("clears ?q= when empty", () => {
      const out = searchToUrl(params("q=oauth&status=Open"), "");
      expect(out.has("q")).toBe(false);
      expect(out.get("status")).toBe("Open");
    });
  });
});
