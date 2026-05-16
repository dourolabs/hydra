// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";

import {
  SIDEBAR_HIDDEN_STORAGE_KEY,
  useSidebarHidden,
} from "./useSidebarHidden";

function mockMatchMedia(matches: boolean) {
  vi.spyOn(window, "matchMedia").mockReturnValue({
    matches,
    media: "",
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => true,
  } as unknown as MediaQueryList);
}

describe("useSidebarHidden", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  afterEach(() => {
    window.localStorage.clear();
    vi.restoreAllMocks();
  });

  it("defaults to visible when no value is stored (desktop viewport)", () => {
    const { result } = renderHook(() => useSidebarHidden());
    expect(result.current.hidden).toBe(false);
  });

  it("defaults to visible on a mobile viewport too (drawer CSS handles off-screen)", () => {
    // The mobile drawer CSS starts the sidebar off-screen via translateX(-100%)
    // when data-sidebar !== "open", so we no longer need React state to track
    // that. Defaulting to false keeps the user from getting stranded with a
    // hidden sidebar after a mobile→desktop resize.
    mockMatchMedia(true);
    const { result } = renderHook(() => useSidebarHidden());
    expect(result.current.hidden).toBe(false);
  });

  it("respects a stored '0' even on a mobile viewport", () => {
    mockMatchMedia(true);
    window.localStorage.setItem(SIDEBAR_HIDDEN_STORAGE_KEY, "0");
    const { result } = renderHook(() => useSidebarHidden());
    expect(result.current.hidden).toBe(false);
  });

  it("rehydrates the hidden state from localStorage on mount", () => {
    window.localStorage.setItem(SIDEBAR_HIDDEN_STORAGE_KEY, "1");
    const { result } = renderHook(() => useSidebarHidden());
    expect(result.current.hidden).toBe(true);
  });

  it("persists hide() to localStorage as '1'", () => {
    const { result } = renderHook(() => useSidebarHidden());
    act(() => {
      result.current.hide();
    });
    expect(result.current.hidden).toBe(true);
    expect(window.localStorage.getItem(SIDEBAR_HIDDEN_STORAGE_KEY)).toBe("1");
  });

  it("persists show() to localStorage as '0'", () => {
    window.localStorage.setItem(SIDEBAR_HIDDEN_STORAGE_KEY, "1");
    const { result } = renderHook(() => useSidebarHidden());
    act(() => {
      result.current.show();
    });
    expect(result.current.hidden).toBe(false);
    expect(window.localStorage.getItem(SIDEBAR_HIDDEN_STORAGE_KEY)).toBe("0");
  });

  it("ignores stored values other than '1'", () => {
    window.localStorage.setItem(SIDEBAR_HIDDEN_STORAGE_KEY, "true");
    const { result } = renderHook(() => useSidebarHidden());
    expect(result.current.hidden).toBe(false);
  });
});
