// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { act, renderHook } from "@testing-library/react";

import {
  PROJECT_COLLAPSE_STORAGE_KEY,
  useProjectCollapseState,
} from "./useProjectCollapseState";

describe("useProjectCollapseState", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  afterEach(() => {
    window.localStorage.clear();
  });

  it("defaults to no projects collapsed when storage is empty", () => {
    const { result } = renderHook(() => useProjectCollapseState());
    expect(result.current.isCollapsed("j-eng")).toBe(false);
    expect(result.current.isCollapsed("j-design")).toBe(false);
  });

  it("rehydrates the collapsed set from localStorage on mount", () => {
    window.localStorage.setItem(
      PROJECT_COLLAPSE_STORAGE_KEY,
      JSON.stringify(["j-eng", "j-design"]),
    );
    const { result } = renderHook(() => useProjectCollapseState());
    expect(result.current.isCollapsed("j-eng")).toBe(true);
    expect(result.current.isCollapsed("j-design")).toBe(true);
    expect(result.current.isCollapsed("j-other")).toBe(false);
  });

  it("toggle adds a project id and persists it to localStorage", () => {
    const { result } = renderHook(() => useProjectCollapseState());
    act(() => {
      result.current.onToggle("j-eng");
    });
    expect(result.current.isCollapsed("j-eng")).toBe(true);
    expect(
      JSON.parse(
        window.localStorage.getItem(PROJECT_COLLAPSE_STORAGE_KEY) ?? "[]",
      ),
    ).toEqual(["j-eng"]);
  });

  it("toggling an already-collapsed project removes it", () => {
    window.localStorage.setItem(
      PROJECT_COLLAPSE_STORAGE_KEY,
      JSON.stringify(["j-eng"]),
    );
    const { result } = renderHook(() => useProjectCollapseState());
    act(() => {
      result.current.onToggle("j-eng");
    });
    expect(result.current.isCollapsed("j-eng")).toBe(false);
    expect(
      JSON.parse(
        window.localStorage.getItem(PROJECT_COLLAPSE_STORAGE_KEY) ?? "[]",
      ),
    ).toEqual([]);
  });

  it("ignores a corrupt stored value and starts empty", () => {
    window.localStorage.setItem(PROJECT_COLLAPSE_STORAGE_KEY, "not json");
    const { result } = renderHook(() => useProjectCollapseState());
    expect(result.current.isCollapsed("j-eng")).toBe(false);
  });

  it("ignores a non-array stored value and starts empty", () => {
    window.localStorage.setItem(
      PROJECT_COLLAPSE_STORAGE_KEY,
      JSON.stringify({ "j-eng": true }),
    );
    const { result } = renderHook(() => useProjectCollapseState());
    expect(result.current.isCollapsed("j-eng")).toBe(false);
  });
});
