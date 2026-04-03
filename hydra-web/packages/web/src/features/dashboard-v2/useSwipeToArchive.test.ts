import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { renderHook } from "@testing-library/react";
import { useSwipeToArchive } from "./useSwipeToArchive";

function createMockElement(): HTMLElement {
  const el = document.createElement("div");
  document.body.appendChild(el);
  return el;
}

function touchStart(el: HTMLElement, clientX: number) {
  el.dispatchEvent(new TouchEvent("touchstart", {
    bubbles: true,
    touches: [{ clientX, clientY: 0 } as Touch],
  }));
}

function touchMove(el: HTMLElement, clientX: number) {
  el.dispatchEvent(new TouchEvent("touchmove", {
    bubbles: true,
    touches: [{ clientX, clientY: 0 } as Touch],
  }));
}

function touchEnd(el: HTMLElement) {
  el.dispatchEvent(new TouchEvent("touchend", { bubbles: true }));
}

describe("useSwipeToArchive", () => {
  let el: HTMLElement;

  beforeEach(() => {
    vi.useFakeTimers();
    el = createMockElement();
  });

  afterEach(() => {
    vi.useRealTimers();
    el.remove();
  });

  it("calls onArchive via setTimeout fallback when transitionend does not fire", () => {
    const onArchive = vi.fn();
    const ref = { current: el };

    renderHook(() =>
      useSwipeToArchive(ref, { onArchive, commitThreshold: 50 }),
    );

    touchStart(el, 200);
    touchMove(el, 100);
    touchEnd(el);

    expect(onArchive).not.toHaveBeenCalled();

    vi.advanceTimersByTime(250);

    expect(onArchive).toHaveBeenCalledTimes(1);
  });

  it("calls onArchive only once when both transitionend and setTimeout fire", () => {
    const onArchive = vi.fn();
    const ref = { current: el };

    renderHook(() =>
      useSwipeToArchive(ref, { onArchive, commitThreshold: 50 }),
    );

    touchStart(el, 200);
    touchMove(el, 100);
    touchEnd(el);

    // Fire transitionend first (jsdom lacks TransitionEvent, use Event with propertyName)
    const transitionEvent = new Event("transitionend", { bubbles: true });
    (transitionEvent as unknown as { propertyName: string }).propertyName = "transform";
    el.dispatchEvent(transitionEvent);

    expect(onArchive).toHaveBeenCalledTimes(1);

    // Then let setTimeout fire
    vi.advanceTimersByTime(250);

    expect(onArchive).toHaveBeenCalledTimes(1);
  });

  it("does not call onArchive when swipe is below threshold (snap back)", () => {
    const onArchive = vi.fn();
    const ref = { current: el };

    renderHook(() =>
      useSwipeToArchive(ref, { onArchive, commitThreshold: 200 }),
    );

    touchStart(el, 200);
    touchMove(el, 160);
    touchEnd(el);

    vi.advanceTimersByTime(500);

    expect(onArchive).not.toHaveBeenCalled();
  });

  it("removes swipeSnapBack class after snap-back transition completes", () => {
    const onArchive = vi.fn();
    const ref = { current: el };

    renderHook(() =>
      useSwipeToArchive(ref, { onArchive, commitThreshold: 200 }),
    );

    touchStart(el, 200);
    touchMove(el, 160);
    touchEnd(el);

    // swipeSnapBack should be present immediately after touch end
    expect(el.classList.length).toBeGreaterThan(0);

    // Fire transitionend to simulate animation completing
    const transitionEvent = new Event("transitionend", { bubbles: true });
    (transitionEvent as unknown as { propertyName: string }).propertyName = "transform";
    el.dispatchEvent(transitionEvent);

    // swipeSnapBack should be removed after transition
    expect(el.className).toBe("");
  });

  it("removes swipeSnapBack class via fallback timeout when transitionend does not fire", () => {
    const onArchive = vi.fn();
    const ref = { current: el };

    renderHook(() =>
      useSwipeToArchive(ref, { onArchive, commitThreshold: 200 }),
    );

    touchStart(el, 200);
    touchMove(el, 160);
    touchEnd(el);

    expect(el.classList.length).toBeGreaterThan(0);

    vi.advanceTimersByTime(250);

    expect(el.className).toBe("");
  });

  it("does not call onArchive on unmount when no swipe was committed", () => {
    const onArchive = vi.fn();
    const ref = { current: el };

    const { unmount } = renderHook(() =>
      useSwipeToArchive(ref, { onArchive, commitThreshold: 50 }),
    );

    unmount();

    expect(onArchive).not.toHaveBeenCalled();
  });

  it("does not fire onArchive when disabled", () => {
    const onArchive = vi.fn();
    const ref = { current: el };

    renderHook(() =>
      useSwipeToArchive(ref, { onArchive, commitThreshold: 50, enabled: false }),
    );

    touchStart(el, 200);
    touchMove(el, 100);
    touchEnd(el);

    vi.advanceTimersByTime(500);

    expect(onArchive).not.toHaveBeenCalled();
  });
});
