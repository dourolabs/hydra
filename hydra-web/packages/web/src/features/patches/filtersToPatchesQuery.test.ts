import { describe, it, expect } from "vitest";
import type { Filter } from "../filters";
import { filtersToPatchesQuery } from "./filtersToPatchesQuery";

function mkFilter(id: string, values: string[], op: "in" | "not_in" = "in"): Filter {
  return { _uid: `t:${id}`, id, op, values };
}

describe("filtersToPatchesQuery", () => {
  it("returns an empty object for no filters and no search", () => {
    expect(
      filtersToPatchesQuery({ filters: [], q: "", extraIds: null }),
    ).toEqual({});
  });

  it("maps the status filter to the server status[] param (multi-select)", () => {
    expect(
      filtersToPatchesQuery({
        filters: [mkFilter("status", ["Open", "Merged"])],
        q: "",
        extraIds: null,
      }),
    ).toEqual({ status: ["Open", "Merged"] });
  });

  it("maps the repository filter to repo_name (single-select)", () => {
    expect(
      filtersToPatchesQuery({
        filters: [mkFilter("repository", ["acme/web-app"])],
        q: "",
        extraIds: null,
      }),
    ).toEqual({ repo_name: "acme/web-app" });
  });

  it("maps the author filter to creator and strips Principal-path prefix", () => {
    expect(
      filtersToPatchesQuery({
        filters: [mkFilter("author", ["users/alice"])],
        q: "",
        extraIds: null,
      }),
    ).toEqual({ creator: "alice" });

    expect(
      filtersToPatchesQuery({
        filters: [mkFilter("author", ["agents/swe"])],
        q: "",
        extraIds: null,
      }),
    ).toEqual({ creator: "swe" });

    // Bare usernames pass through unchanged.
    expect(
      filtersToPatchesQuery({
        filters: [mkFilter("author", ["alice"])],
        q: "",
        extraIds: null,
      }),
    ).toEqual({ creator: "alice" });
  });

  it("threads the free-text q through (trimmed)", () => {
    expect(
      filtersToPatchesQuery({ filters: [], q: "  oauth  ", extraIds: null }),
    ).toEqual({ q: "oauth" });
  });

  it("omits q when it is whitespace-only", () => {
    expect(
      filtersToPatchesQuery({ filters: [], q: "   ", extraIds: null }),
    ).toEqual({});
  });

  it("maps relation-resolved extraIds onto the ids server param", () => {
    expect(
      filtersToPatchesQuery({
        filters: [],
        q: "",
        extraIds: ["p-aaa", "p-bbb"],
      }),
    ).toEqual({ ids: "p-aaa,p-bbb" });
  });

  it("uses a sentinel id when an active relation matched no patches", () => {
    // Active relation filter with no matches must still narrow the server
    // response to zero rows; passing an empty `ids=` would no-op.
    expect(
      filtersToPatchesQuery({ filters: [], q: "", extraIds: [] }),
    ).toEqual({ ids: "__no_match__" });
  });

  it("drops not_in filters (no server param can express negation today)", () => {
    expect(
      filtersToPatchesQuery({
        filters: [mkFilter("status", ["Open"], "not_in")],
        q: "",
        extraIds: null,
      }),
    ).toEqual({});
  });

  it("drops relation filter ids (resolved upstream into extraIds)", () => {
    expect(
      filtersToPatchesQuery({
        filters: [mkFilter("relatedIssue", ["i-abc"])],
        q: "",
        extraIds: null,
      }),
    ).toEqual({});
  });

  it("combines status + repo + author + q + ids in one mapping", () => {
    expect(
      filtersToPatchesQuery({
        filters: [
          mkFilter("status", ["Open"]),
          mkFilter("repository", ["acme/web-app"]),
          mkFilter("author", ["users/alice"]),
        ],
        q: "oauth",
        extraIds: ["p-1", "p-2"],
      }),
    ).toEqual({
      status: ["Open"],
      repo_name: "acme/web-app",
      creator: "alice",
      q: "oauth",
      ids: "p-1,p-2",
    });
  });
});
