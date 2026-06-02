// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import { render, cleanup, fireEvent, act } from "@testing-library/react";
import { useState } from "react";
import { FilterBar } from "../FilterBar";
import type { Filter, FilterDefinitions } from "../types";

// Stub `@hydra/ui` so the test doesn't drag in the real component graph or
// CSS-Modules transforms. Icons are simple spans with a data-testid.
vi.mock("@hydra/ui", () => ({
  Icons: new Proxy(
    {},
    {
      get: () => () => <span data-testid="icon" />,
    },
  ),
}));

interface Item {
  id: string;
  status: string;
}

const ITEMS: Item[] = [
  { id: "1", status: "open" },
  { id: "2", status: "open" },
  { id: "3", status: "closed" },
];

const DEFS: FilterDefinitions<Item> = {
  status: {
    label: "Status",
    icon: () => null,
    group: "properties",
    kind: "enum",
    options: [
      { value: "open", label: "Open", chip: <span>Open</span>, render: <span>Open</span> },
      { value: "closed", label: "Closed", chip: <span>Closed</span>, render: <span>Closed</span> },
    ],
    apply: (item, f) => f.values.includes(item.status),
  },
  flavor: {
    label: "Flavor",
    icon: () => null,
    group: "properties",
    kind: "enum",
    options: [
      { value: "a", label: "A", chip: <span>A</span>, render: <span>A</span> },
      { value: "b", label: "B", chip: <span>B</span>, render: <span>B</span> },
      { value: "c", label: "C", chip: <span>C</span>, render: <span>C</span> },
    ],
    apply: () => true,
  },
};

function Harness({ initial }: { initial: Filter[] }) {
  const [filters, setFilters] = useState<Filter[]>(initial);
  const matched = filters.reduce((acc, f) => {
    const def = DEFS[f.id];
    if (!def || f.values.length === 0) return acc;
    return acc.filter((item) => {
      const r = def.apply(item, f);
      return f.op === "not_in" ? !r : r;
    });
  }, ITEMS);
  return (
    <FilterBar
      filters={filters}
      setFilters={setFilters}
      definitions={DEFS}
      count={matched.length}
      total={ITEMS.length}
    />
  );
}

function chip(values: string[], id = "status"): Filter {
  return { _uid: `u-${id}-${values.join("-")}`, id, op: "in", values };
}

afterEach(() => cleanup());

describe("FilterBar", () => {
  it("renders '{total} results' when there are no active filters", () => {
    const { getByTestId } = render(<Harness initial={[]} />);
    expect(getByTestId("filter-bar-summary").textContent).toBe("3 results");
  });

  it("renders '{count} of {total}' when an active filter narrows results", () => {
    const { getByTestId } = render(<Harness initial={[chip(["open"])]} />);
    expect(getByTestId("filter-bar-summary").textContent).toBe("2 of 3");
  });

  it("renders one chip per filter and the same definition can appear twice", () => {
    const { container } = render(
      <Harness initial={[chip(["open"]), chip(["closed"])]} />,
    );
    const chips = container.querySelectorAll('[data-testid^="filter-chip-"]');
    expect(chips.length).toBe(2);
  });

  it("Clear all empties the filter set", () => {
    const { getByTestId } = render(<Harness initial={[chip(["open"])]} />);
    act(() => {
      fireEvent.click(getByTestId("filter-bar-clear-all"));
    });
    expect(getByTestId("filter-bar-summary").textContent).toBe("3 results");
  });

  it("Clear all is hidden when no filters are active", () => {
    const { queryByTestId } = render(<Harness initial={[]} />);
    expect(queryByTestId("filter-bar-clear-all")).toBeNull();
  });

  it("renders +N overflow when a chip carries more than two selected values", () => {
    const { container } = render(
      <Harness initial={[{ ...chip(["a", "b", "c"], "flavor") }]} />,
    );
    expect(container.textContent).toContain("+1");
  });
});
