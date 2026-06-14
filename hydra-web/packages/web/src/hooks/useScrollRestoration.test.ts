// @vitest-environment jsdom
import { describe, it, expect, beforeEach, vi } from "vitest";
import { renderHook } from "@testing-library/react";
import { createRef } from "react";
import { useScrollRestoration } from "./useScrollRestoration";

function makeScroller(initial: { left?: number; top?: number } = {}) {
  const el = document.createElement("div");
  el.scrollLeft = initial.left ?? 0;
  el.scrollTop = initial.top ?? 0;
  return el;
}

describe("useScrollRestoration", () => {
  beforeEach(() => {
    window.sessionStorage.clear();
  });

  it("restores scrollLeft and scrollTop from sessionStorage on mount", () => {
    window.sessionStorage.setItem(
      "k",
      JSON.stringify({ left: 320, top: 110 }),
    );
    const el = makeScroller();
    const ref = createRef<HTMLDivElement>();
    (ref as { current: HTMLDivElement }).current = el;

    renderHook(() => useScrollRestoration("k", ref));

    expect(el.scrollLeft).toBe(320);
    expect(el.scrollTop).toBe(110);
  });

  it("persists the current scroll position on unmount", () => {
    const el = makeScroller({ left: 540, top: 12 });
    const ref = createRef<HTMLDivElement>();
    (ref as { current: HTMLDivElement }).current = el;

    const { unmount } = renderHook(() => useScrollRestoration("k", ref));
    unmount();

    const raw = window.sessionStorage.getItem("k");
    expect(raw).not.toBeNull();
    expect(JSON.parse(raw!)).toEqual({ left: 540, top: 12 });
  });

  it("is a no-op when key is null", () => {
    window.sessionStorage.setItem(
      "k",
      JSON.stringify({ left: 999, top: 999 }),
    );
    const el = makeScroller();
    const ref = createRef<HTMLDivElement>();
    (ref as { current: HTMLDivElement }).current = el;

    const { unmount } = renderHook(() => useScrollRestoration(null, ref));

    expect(el.scrollLeft).toBe(0);
    expect(el.scrollTop).toBe(0);
    unmount();
    // No write should happen for a null key.
    expect(window.sessionStorage.getItem("null")).toBeNull();
  });

  it("survives missing ref (does not throw)", () => {
    const ref = createRef<HTMLDivElement>();
    expect(() =>
      renderHook(() => useScrollRestoration("k", ref)),
    ).not.toThrow();
  });

  it("survives malformed sessionStorage payload", () => {
    window.sessionStorage.setItem("k", "{not json");
    const el = makeScroller();
    const ref = createRef<HTMLDivElement>();
    (ref as { current: HTMLDivElement }).current = el;

    expect(() =>
      renderHook(() => useScrollRestoration("k", ref)),
    ).not.toThrow();
    expect(el.scrollLeft).toBe(0);
  });

  it("saves on scroll (rAF coalesced)", async () => {
    const rafCalls: FrameRequestCallback[] = [];
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((cb) => {
      rafCalls.push(cb);
      return 1;
    });
    const el = makeScroller();
    const ref = createRef<HTMLDivElement>();
    (ref as { current: HTMLDivElement }).current = el;

    renderHook(() => useScrollRestoration("k", ref));

    el.scrollLeft = 200;
    el.dispatchEvent(new Event("scroll"));
    el.scrollLeft = 300;
    el.dispatchEvent(new Event("scroll"));

    // The second scroll should not enqueue a second rAF — coalesced.
    expect(rafCalls.length).toBe(1);
    rafCalls[0](performance.now());

    expect(JSON.parse(window.sessionStorage.getItem("k")!)).toEqual({
      left: 300,
      top: 0,
    });
  });
});
