import { describe, it, expect } from "vitest";
import { filtersToIssuesQuery } from "../filtersToIssuesQuery";
import type { Filter } from "../../filters";

function chip(id: string, value: string): Filter {
  return { _uid: `u-${id}-${value}`, id, op: "in", values: [value] };
}

describe("filtersToIssuesQuery — project_id mapping", () => {
  it("maps a project chip to `project_id` on the server query", () => {
    const out = filtersToIssuesQuery({
      filters: [chip("project", "j-engv2")],
      q: "",
      extraIds: null,
    });
    expect(out.project_id).toBe("j-engv2");
  });

  it("passes status through unchanged when both project + status chips are set", () => {
    const out = filtersToIssuesQuery({
      filters: [chip("project", "j-engv2"), chip("status", "inbox")],
      q: "",
      extraIds: null,
    });
    expect(out.project_id).toBe("j-engv2");
    expect(out.status).toBe("inbox");
  });
});
