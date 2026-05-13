// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { act, renderHook } from "@testing-library/react";

import {
  SIDEBAR_HIDDEN_STORAGE_KEY,
  useSidebarHidden,
} from "./useSidebarHidden";

describe("useSidebarHidden", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  afterEach(() => {
    window.localStorage.clear();
  });

  it("defaults to visible when no value is stored", () => {
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
