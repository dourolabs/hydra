// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";

vi.mock("./FloatingActionButton.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

type ChangeListener = (e: MediaQueryListEvent) => void;
function mockMatchMedia(matches: boolean) {
  const listeners: ChangeListener[] = [];
  const mql = {
    matches,
    media: "",
    onchange: null,
    addEventListener: (event: string, handler: ChangeListener) => {
      if (event === "change") listeners.push(handler);
    },
    removeEventListener: (event: string, handler: ChangeListener) => {
      if (event === "change") {
        const idx = listeners.indexOf(handler);
        if (idx !== -1) listeners.splice(idx, 1);
      }
    },
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => true,
  };
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    writable: true,
    value: () => mql as unknown as MediaQueryList,
  });
}

const { FloatingActionButton } = await import("./FloatingActionButton");

afterEach(() => {
  cleanup();
});

describe("FloatingActionButton", () => {
  describe("on mobile", () => {
    beforeEach(() => mockMatchMedia(true));

    it("renders the button with the provided aria-label and icon", () => {
      render(
        <FloatingActionButton
          icon={<span data-testid="fab-icon" />}
          label="New issue"
          onClick={() => {}}
        />,
      );
      const fab = screen.getByRole("button", { name: "New issue" });
      expect(fab).toBeTruthy();
      expect(screen.getByTestId("fab-icon")).toBeTruthy();
    });

    it("invokes onClick when clicked", () => {
      const onClick = vi.fn();
      render(
        <FloatingActionButton icon={<span />} label="New issue" onClick={onClick} />,
      );
      fireEvent.click(screen.getByRole("button", { name: "New issue" }));
      expect(onClick).toHaveBeenCalledTimes(1);
    });

    it("uses the provided testId", () => {
      render(
        <FloatingActionButton
          icon={<span />}
          label="New issue"
          onClick={() => {}}
          testId="my-fab"
        />,
      );
      expect(screen.getByTestId("my-fab")).toBeTruthy();
    });
  });

  describe("on desktop", () => {
    beforeEach(() => mockMatchMedia(false));

    it("does not render", () => {
      render(
        <FloatingActionButton
          icon={<span data-testid="fab-icon" />}
          label="New issue"
          onClick={() => {}}
        />,
      );
      expect(screen.queryByRole("button", { name: "New issue" })).toBeNull();
      expect(screen.queryByTestId("fab-icon")).toBeNull();
    });
  });
});
