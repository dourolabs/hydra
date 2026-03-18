import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useMediaQuery } from "./useMediaQuery";

type ChangeListener = (e: MediaQueryListEvent) => void;

function createMockMediaQueryList(initialMatches: boolean) {
  const listeners: ChangeListener[] = [];
  const mql = {
    matches: initialMatches,
    addEventListener: vi.fn((event: string, handler: ChangeListener) => {
      if (event === "change") listeners.push(handler);
    }),
    removeEventListener: vi.fn((event: string, handler: ChangeListener) => {
      if (event === "change") {
        const idx = listeners.indexOf(handler);
        if (idx !== -1) listeners.splice(idx, 1);
      }
    }),
    fireChange(matches: boolean) {
      mql.matches = matches;
      for (const listener of listeners) {
        listener({ matches } as MediaQueryListEvent);
      }
    },
  };
  return mql;
}

describe("useMediaQuery", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("returns true when the query initially matches", () => {
    const mql = createMockMediaQueryList(true);
    vi.spyOn(window, "matchMedia").mockReturnValue(
      mql as unknown as MediaQueryList,
    );

    const { result } = renderHook(() => useMediaQuery("(min-width: 768px)"));

    expect(result.current).toBe(true);
  });

  it("returns false when the query does not initially match", () => {
    const mql = createMockMediaQueryList(false);
    vi.spyOn(window, "matchMedia").mockReturnValue(
      mql as unknown as MediaQueryList,
    );

    const { result } = renderHook(() => useMediaQuery("(min-width: 768px)"));

    expect(result.current).toBe(false);
  });

  it("updates to true when a change event fires with matches: true", () => {
    const mql = createMockMediaQueryList(false);
    vi.spyOn(window, "matchMedia").mockReturnValue(
      mql as unknown as MediaQueryList,
    );

    const { result } = renderHook(() => useMediaQuery("(min-width: 768px)"));
    expect(result.current).toBe(false);

    act(() => {
      mql.fireChange(true);
    });

    expect(result.current).toBe(true);
  });

  it("updates to false when a change event fires with matches: false", () => {
    const mql = createMockMediaQueryList(true);
    vi.spyOn(window, "matchMedia").mockReturnValue(
      mql as unknown as MediaQueryList,
    );

    const { result } = renderHook(() => useMediaQuery("(min-width: 768px)"));
    expect(result.current).toBe(true);

    act(() => {
      mql.fireChange(false);
    });

    expect(result.current).toBe(false);
  });

  it("cleans up the event listener on unmount", () => {
    const mql = createMockMediaQueryList(false);
    vi.spyOn(window, "matchMedia").mockReturnValue(
      mql as unknown as MediaQueryList,
    );

    const { unmount } = renderHook(() => useMediaQuery("(min-width: 768px)"));

    expect(mql.addEventListener).toHaveBeenCalledWith(
      "change",
      expect.any(Function),
    );

    unmount();

    expect(mql.removeEventListener).toHaveBeenCalledWith(
      "change",
      expect.any(Function),
    );
    // Verify the same handler was used for add and remove
    const addedHandler = mql.addEventListener.mock.calls[0][1];
    const removedHandler = mql.removeEventListener.mock.calls[0][1];
    expect(addedHandler).toBe(removedHandler);
  });

  it("re-subscribes when the query prop changes", () => {
    const mqlNarrow = createMockMediaQueryList(true);
    const mqlWide = createMockMediaQueryList(false);

    vi.spyOn(window, "matchMedia").mockImplementation((query: string) => {
      if (query === "(min-width: 768px)") {
        return mqlNarrow as unknown as MediaQueryList;
      }
      return mqlWide as unknown as MediaQueryList;
    });

    const { result, rerender } = renderHook(
      ({ query }: { query: string }) => useMediaQuery(query),
      { initialProps: { query: "(min-width: 768px)" } },
    );

    expect(result.current).toBe(true);

    // The old listener should be removed when query changes
    rerender({ query: "(min-width: 1024px)" });

    expect(mqlNarrow.removeEventListener).toHaveBeenCalledWith(
      "change",
      expect.any(Function),
    );
    expect(mqlWide.addEventListener).toHaveBeenCalledWith(
      "change",
      expect.any(Function),
    );
    expect(result.current).toBe(false);
  });
});
