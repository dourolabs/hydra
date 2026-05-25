// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { renderHook } from "@testing-library/react";

const isMobileMock = vi.fn<() => boolean>(() => false);
vi.mock("../../hooks/useIsMobile", () => ({
  useIsMobile: () => isMobileMock(),
  MOBILE_MEDIA_QUERY: "(max-width: 768px)",
}));

const { useGlobalSearchShortcut } = await import("./useGlobalSearchShortcut");

function fireKey(init: KeyboardEventInit) {
  window.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, ...init }));
}

describe("useGlobalSearchShortcut", () => {
  beforeEach(() => {
    isMobileMock.mockReturnValue(false);
  });

  afterEach(() => {
    vi.clearAllMocks();
    isMobileMock.mockReturnValue(false);
  });

  it("calls toggle on Cmd+K on desktop", () => {
    const toggle = vi.fn();
    renderHook(() => useGlobalSearchShortcut(toggle));

    fireKey({ key: "k", metaKey: true });

    expect(toggle).toHaveBeenCalledTimes(1);
  });

  it("calls toggle on Ctrl+K on desktop", () => {
    const toggle = vi.fn();
    renderHook(() => useGlobalSearchShortcut(toggle));

    fireKey({ key: "k", ctrlKey: true });

    expect(toggle).toHaveBeenCalledTimes(1);
  });

  it("does NOT register the listener on mobile", () => {
    isMobileMock.mockReturnValue(true);
    const toggle = vi.fn();
    renderHook(() => useGlobalSearchShortcut(toggle));

    fireKey({ key: "k", metaKey: true });
    fireKey({ key: "k", ctrlKey: true });

    expect(toggle).not.toHaveBeenCalled();
  });

  it("ignores plain k without a modifier on desktop", () => {
    const toggle = vi.fn();
    renderHook(() => useGlobalSearchShortcut(toggle));

    fireKey({ key: "k" });

    expect(toggle).not.toHaveBeenCalled();
  });
});
