import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { act, renderHook } from "@testing-library/react";

import {
  clearConversationDraft,
  conversationDraftKey,
  useConversationDraft,
} from "../useConversationDraft";

describe("useConversationDraft", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  afterEach(() => {
    window.localStorage.clear();
  });

  it("returns an empty draft for a conversation that has none stored", () => {
    const { result } = renderHook(() => useConversationDraft("c-1"));
    expect(result.current.value).toBe("");
  });

  it("rehydrates the stored draft on mount", () => {
    window.localStorage.setItem(conversationDraftKey("c-1"), "remember me");
    const { result } = renderHook(() => useConversationDraft("c-1"));
    expect(result.current.value).toBe("remember me");
  });

  it("persists updates and removes the entry when set to empty", () => {
    const { result } = renderHook(() => useConversationDraft("c-1"));

    act(() => result.current.setValue("hello"));
    expect(result.current.value).toBe("hello");
    expect(window.localStorage.getItem(conversationDraftKey("c-1"))).toBe("hello");

    act(() => result.current.setValue(""));
    expect(window.localStorage.getItem(conversationDraftKey("c-1"))).toBeNull();
  });

  it("clear() wipes both state and stored value", () => {
    window.localStorage.setItem(conversationDraftKey("c-1"), "queued");
    const { result } = renderHook(() => useConversationDraft("c-1"));

    act(() => result.current.clear());

    expect(result.current.value).toBe("");
    expect(window.localStorage.getItem(conversationDraftKey("c-1"))).toBeNull();
  });

  it("reloads the draft when the conversation id changes", () => {
    window.localStorage.setItem(conversationDraftKey("c-1"), "for one");
    window.localStorage.setItem(conversationDraftKey("c-2"), "for two");

    const { result, rerender } = renderHook(({ id }) => useConversationDraft(id), {
      initialProps: { id: "c-1" },
    });
    expect(result.current.value).toBe("for one");

    rerender({ id: "c-2" });
    expect(result.current.value).toBe("for two");
  });

  it("clearConversationDraft removes only the targeted conversation's entry", () => {
    window.localStorage.setItem(conversationDraftKey("c-1"), "one");
    window.localStorage.setItem(conversationDraftKey("c-2"), "two");

    clearConversationDraft("c-1");

    expect(window.localStorage.getItem(conversationDraftKey("c-1"))).toBeNull();
    expect(window.localStorage.getItem(conversationDraftKey("c-2"))).toBe("two");
  });
});
