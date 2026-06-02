import { describe, it, expect } from "vitest";
import { applyFilters } from "../applyFilters";
import type { Filter, FilterDefinitions } from "../types";

interface Item {
  id: string;
  status: string;
  type: string;
}

const ITEMS: Item[] = [
  { id: "1", status: "open", type: "bug" },
  { id: "2", status: "open", type: "feature" },
  { id: "3", status: "closed", type: "bug" },
  { id: "4", status: "in-progress", type: "task" },
];

const DEFS: FilterDefinitions<Item> = {
  status: {
    label: "Status",
    icon: () => null,
    group: "properties",
    kind: "enum",
    options: [],
    apply: (item, f) => f.values.includes(item.status),
  },
  type: {
    label: "Type",
    icon: () => null,
    group: "properties",
    kind: "enum",
    options: [],
    apply: (item, f) => f.values.includes(item.type),
  },
};

function f(id: string, op: Filter["op"], values: string[]): Filter {
  return { _uid: `uid-${id}-${values.join("-")}`, id, op, values };
}

describe("applyFilters", () => {
  it("returns the same array reference when no filters are passed", () => {
    const out = applyFilters(ITEMS, [], DEFS);
    expect(out).toBe(ITEMS);
  });

  it("returns the same array reference when every filter has empty values", () => {
    const out = applyFilters(ITEMS, [f("status", "in", [])], DEFS);
    expect(out).toBe(ITEMS);
  });

  it("filters on a single 'in' filter", () => {
    const out = applyFilters(ITEMS, [f("status", "in", ["open"])], DEFS);
    expect(out.map((x) => x.id)).toEqual(["1", "2"]);
  });

  it("ANDs across multiple filters", () => {
    const out = applyFilters(
      ITEMS,
      [f("status", "in", ["open"]), f("type", "in", ["bug"])],
      DEFS,
    );
    expect(out.map((x) => x.id)).toEqual(["1"]);
  });

  it("treats values as union within a single filter", () => {
    const out = applyFilters(
      ITEMS,
      [f("status", "in", ["open", "in-progress"])],
      DEFS,
    );
    expect(out.map((x) => x.id)).toEqual(["1", "2", "4"]);
  });

  it("negates membership under op 'not_in'", () => {
    const out = applyFilters(ITEMS, [f("status", "not_in", ["open"])], DEFS);
    expect(out.map((x) => x.id)).toEqual(["3", "4"]);
  });

  it("skips a filter whose id is not in the definitions", () => {
    const out = applyFilters(
      ITEMS,
      [f("unknown", "in", ["something"]), f("status", "in", ["open"])],
      DEFS,
    );
    expect(out.map((x) => x.id)).toEqual(["1", "2"]);
  });
});
