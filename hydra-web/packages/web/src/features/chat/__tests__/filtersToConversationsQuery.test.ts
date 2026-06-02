import { describe, it, expect } from "vitest";
import { filtersToConversationsQuery } from "../filtersToConversationsQuery";
import type { Filter } from "../../filters";

function mkFilter(id: string, values: string[], op: "in" | "not_in" = "in"): Filter {
  return { _uid: `t:${id}`, id, op, values };
}

describe("filtersToConversationsQuery", () => {
  it("returns an empty object when no filters or search are set", () => {
    expect(filtersToConversationsQuery({ filters: [], q: "" })).toEqual({});
  });

  it("maps status filter to ?status=", () => {
    const out = filtersToConversationsQuery({
      filters: [mkFilter("status", ["active"])],
      q: "",
    });
    expect(out).toEqual({ status: "active" });
  });

  it("maps creator filter, stripping users/ prefix", () => {
    const out = filtersToConversationsQuery({
      filters: [mkFilter("creator", ["users/alice"])],
      q: "",
    });
    expect(out).toEqual({ creator: "alice" });
  });

  it("maps creator filter, stripping agents/ prefix", () => {
    const out = filtersToConversationsQuery({
      filters: [mkFilter("creator", ["agents/swe"])],
      q: "",
    });
    expect(out).toEqual({ creator: "swe" });
  });

  it("passes a bare creator value through unchanged", () => {
    const out = filtersToConversationsQuery({
      filters: [mkFilter("creator", ["bob"])],
      q: "",
    });
    expect(out).toEqual({ creator: "bob" });
  });

  it("includes trimmed q when set", () => {
    expect(
      filtersToConversationsQuery({ filters: [], q: "  hello  " }),
    ).toEqual({ q: "hello" });
  });

  it("omits q when blank or whitespace-only", () => {
    expect(filtersToConversationsQuery({ filters: [], q: "   " })).toEqual({});
  });

  it("combines status, creator, and q into a single query", () => {
    const out = filtersToConversationsQuery({
      filters: [
        mkFilter("status", ["idle"]),
        mkFilter("creator", ["users/alice"]),
      ],
      q: "deploy",
    });
    expect(out).toEqual({ status: "idle", creator: "alice", q: "deploy" });
  });

  it("drops not_in filters (server cannot express negation)", () => {
    const out = filtersToConversationsQuery({
      filters: [mkFilter("status", ["closed"], "not_in")],
      q: "",
    });
    expect(out).toEqual({});
  });

  it("drops filters with empty value lists", () => {
    const out = filtersToConversationsQuery({
      filters: [mkFilter("status", [])],
      q: "",
    });
    expect(out).toEqual({});
  });

  it("ignores unknown filter ids", () => {
    const out = filtersToConversationsQuery({
      filters: [mkFilter("relatedIssue", ["i-aaa"])],
      q: "",
    });
    expect(out).toEqual({});
  });
});
