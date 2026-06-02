import { describe, it, expect } from "vitest";
import { filtersToSessionsQuery } from "../filtersToSessionsQuery";
import type { Filter } from "../../filters";

function f(id: string, values: string[], op: Filter["op"] = "in"): Filter {
  return { _uid: `uid-${id}`, id, op, values };
}

describe("filtersToSessionsQuery", () => {
  it("returns an empty query for empty filters and no q", () => {
    const q = filtersToSessionsQuery({
      filters: [],
      q: "",
      patchIssueIds: null,
    });
    expect(q).toEqual({});
  });

  it("joins multi-select status values into a CSV string", () => {
    const q = filtersToSessionsQuery({
      filters: [f("status", ["running", "pending"])],
      q: "",
      patchIssueIds: null,
    });
    expect(q.status).toBe("running,pending");
  });

  it("strips users/ prefix from creator value", () => {
    const q = filtersToSessionsQuery({
      filters: [f("creator", ["users/alice"])],
      q: "",
      patchIssueIds: null,
    });
    expect(q.creator).toBe("alice");
  });

  it("strips agents/ prefix from creator value", () => {
    const q = filtersToSessionsQuery({
      filters: [f("creator", ["agents/claude"])],
      q: "",
      patchIssueIds: null,
    });
    expect(q.creator).toBe("claude");
  });

  it("maps relatedIssue values directly to spawned_from_ids", () => {
    const q = filtersToSessionsQuery({
      filters: [f("relatedIssue", ["i-1", "i-2"])],
      q: "",
      patchIssueIds: null,
    });
    expect(q.spawned_from_ids).toBe("i-1,i-2");
  });

  it("maps relatedChat single value to conversation_id", () => {
    const q = filtersToSessionsQuery({
      filters: [f("relatedChat", ["c-42"])],
      q: "",
      patchIssueIds: null,
    });
    expect(q.conversation_id).toBe("c-42");
  });

  it("uses patchIssueIds for spawned_from_ids when relatedPatch is the only relation filter", () => {
    const q = filtersToSessionsQuery({
      filters: [],
      q: "",
      patchIssueIds: ["i-a", "i-b"],
    });
    expect(q.spawned_from_ids).toBe("i-a,i-b");
  });

  it("intersects relatedIssue values with patchIssueIds when both are active", () => {
    const q = filtersToSessionsQuery({
      filters: [f("relatedIssue", ["i-1", "i-2", "i-3"])],
      q: "",
      patchIssueIds: ["i-2", "i-3", "i-9"],
    });
    // AND semantics: intersection of relatedIssue values and patchIssueIds.
    expect(q.spawned_from_ids).toBe("i-2,i-3");
  });

  it("falls back to a sentinel id when relatedPatch resolves to no issues", () => {
    const q = filtersToSessionsQuery({
      filters: [],
      q: "",
      patchIssueIds: [],
    });
    expect(q.spawned_from_ids).toBe("i-__no_match__");
  });

  it("falls back to a sentinel id when the relatedIssue ∩ patchIssueIds intersection is empty", () => {
    const q = filtersToSessionsQuery({
      filters: [f("relatedIssue", ["i-1"])],
      q: "",
      patchIssueIds: ["i-2"],
    });
    expect(q.spawned_from_ids).toBe("i-__no_match__");
  });

  it("forwards a trimmed q value", () => {
    const q = filtersToSessionsQuery({
      filters: [],
      q: "  deploy  ",
      patchIssueIds: null,
    });
    expect(q.q).toBe("deploy");
  });

  it("drops `not_in` filters silently", () => {
    const q = filtersToSessionsQuery({
      filters: [f("status", ["running"], "not_in")],
      q: "",
      patchIssueIds: null,
    });
    expect(q).toEqual({});
  });

  it("drops empty-value filters", () => {
    const q = filtersToSessionsQuery({
      filters: [f("status", [])],
      q: "",
      patchIssueIds: null,
    });
    expect(q).toEqual({});
  });

  it("combines status, creator, and q into a single query", () => {
    const q = filtersToSessionsQuery({
      filters: [
        f("status", ["running"]),
        f("creator", ["users/alice"]),
      ],
      q: "deploy",
      patchIssueIds: null,
    });
    expect(q).toEqual({
      status: "running",
      creator: "alice",
      q: "deploy",
    });
  });
});
