// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import { render, cleanup } from "@testing-library/react";
import { FilterChip } from "../FilterChip";
import type { Filter, FilterDefinition } from "../types";

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
}

function makeDef(
  notInSupported: boolean | undefined,
): FilterDefinition<Item> {
  return {
    label: "Status",
    icon: () => null,
    group: "properties",
    kind: "enum",
    notInSupported,
    options: [
      { value: "open", label: "Open", chip: <span>Open</span>, render: <span>Open</span> },
      { value: "closed", label: "Closed", chip: <span>Closed</span>, render: <span>Closed</span> },
    ],
    apply: () => true,
  };
}

function makeFilter(op: Filter["op"]): Filter {
  return { _uid: "u1", id: "status", op, values: ["open"] };
}

afterEach(() => cleanup());

describe("FilterChip op-prefix", () => {
  it("omits the 'is' / 'is not' prefix when notInSupported is undefined", () => {
    const { container } = render(
      <FilterChip
        filter={makeFilter("in")}
        definition={makeDef(undefined)}
        open={false}
        onOpen={() => {}}
        onRemove={() => {}}
      />,
    );
    expect(container.textContent).not.toContain("is");
    expect(container.textContent).not.toContain("is not");
  });

  it("omits the prefix when notInSupported is explicitly false", () => {
    const { container } = render(
      <FilterChip
        filter={makeFilter("in")}
        definition={makeDef(false)}
        open={false}
        onOpen={() => {}}
        onRemove={() => {}}
      />,
    );
    expect(container.textContent).not.toContain("is");
  });

  it("renders 'is' when notInSupported is true and op is 'in'", () => {
    const { container } = render(
      <FilterChip
        filter={makeFilter("in")}
        definition={makeDef(true)}
        open={false}
        onOpen={() => {}}
        onRemove={() => {}}
      />,
    );
    expect(container.textContent).toContain("is");
    expect(container.textContent).not.toContain("is not");
  });

  it("renders 'is not' when notInSupported is true and op is 'not_in'", () => {
    const { container } = render(
      <FilterChip
        filter={makeFilter("not_in")}
        definition={makeDef(true)}
        open={false}
        onOpen={() => {}}
        onRemove={() => {}}
      />,
    );
    expect(container.textContent).toContain("is not");
  });
});
