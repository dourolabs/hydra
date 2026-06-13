// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import { useState } from "react";
import {
  render,
  cleanup,
  fireEvent,
  screen,
  act,
} from "@testing-library/react";

vi.mock("../CollapsibleSearch/CollapsibleSearch.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("@hydra/ui", () => ({
  Icons: {
    IconSearch: () => <span data-testid="icon-search" />,
    IconX: () => <span data-testid="icon-x" />,
  },
}));

let mobileMatches = false;
vi.mock("../../hooks/useMediaQuery", () => ({
  useMediaQuery: () => mobileMatches,
}));

const { CollapsibleSearch } = await import(
  "../CollapsibleSearch/CollapsibleSearch"
);

function Harness({ initial = "" }: { initial?: string }) {
  const [value, setValue] = useState(initial);
  return (
    <CollapsibleSearch
      value={value}
      onChange={setValue}
      placeholder="Search…"
      ariaLabel="Search"
      testId="cs"
    />
  );
}

describe("CollapsibleSearch", () => {
  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
    mobileMatches = false;
  });

  describe("desktop", () => {
    it("renders the input inline regardless of value", () => {
      render(<Harness />);
      expect(screen.getByTestId("cs")).toBeDefined();
      expect(screen.queryByTestId("cs-toggle")).toBeNull();
    });

    it("does not render a clear button on desktop", () => {
      render(<Harness initial="foo" />);
      expect(screen.queryByTestId("cs-clear")).toBeNull();
    });
  });

  describe("mobile", () => {
    it("collapses to the icon button when value is empty", () => {
      mobileMatches = true;
      render(<Harness />);
      expect(screen.getByTestId("cs-toggle")).toBeDefined();
      expect(screen.queryByTestId("cs")).toBeNull();
    });

    it("expands to the input when value is non-empty on mount", () => {
      mobileMatches = true;
      render(<Harness initial="foo" />);
      expect(screen.getByTestId("cs")).toBeDefined();
      expect(screen.queryByTestId("cs-toggle")).toBeNull();
    });

    it("expands and focuses the input when the icon button is tapped", () => {
      mobileMatches = true;
      render(<Harness />);
      act(() => {
        fireEvent.click(screen.getByTestId("cs-toggle"));
      });
      const input = screen.getByTestId("cs") as HTMLInputElement;
      expect(input).toBeDefined();
      expect(document.activeElement).toBe(input);
    });

    it("clear button clears the value and collapses back to the icon", () => {
      mobileMatches = true;
      render(<Harness initial="foo" />);
      const input = screen.getByTestId("cs") as HTMLInputElement;
      expect(input.value).toBe("foo");
      act(() => {
        fireEvent.click(screen.getByTestId("cs-clear"));
      });
      expect(screen.queryByTestId("cs")).toBeNull();
      expect(screen.getByTestId("cs-toggle")).toBeDefined();
    });

    it("Escape collapses without clearing the value", () => {
      mobileMatches = true;
      render(<Harness initial="foo" />);
      const input = screen.getByTestId("cs") as HTMLInputElement;
      act(() => {
        fireEvent.keyDown(input, { key: "Escape" });
      });
      // Value is preserved on the page (parent state still holds "foo"),
      // but the input is hidden behind the icon button.
      expect(screen.queryByTestId("cs")).toBeNull();
      expect(screen.getByTestId("cs-toggle")).toBeDefined();
    });
  });
});
