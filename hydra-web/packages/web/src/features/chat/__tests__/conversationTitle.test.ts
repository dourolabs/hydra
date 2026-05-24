import { describe, it, expect } from "vitest";
import type { ConversationSummary } from "@hydra/api";

import { conversationTitle } from "../conversationTitle";

function summary(overrides: Partial<ConversationSummary> = {}): ConversationSummary {
  return {
    conversation_id: "c-1",
    title: null,
    agent_name: null,
    status: "active",
    event_count: 0,
    last_event_preview: null,
    creator: "alice",
    created_at: "2026-05-24T00:00:00Z",
    updated_at: "2026-05-24T00:00:00Z",
    ...overrides,
  };
}

describe("conversationTitle", () => {
  it("returns the explicit title when present", () => {
    expect(
      conversationTitle(
        summary({ title: "Casual chat with Alice", last_event_preview: "User: hello" }),
      ),
    ).toBe("Casual chat with Alice");
  });

  it("returns a chat-message preview when no title is set", () => {
    expect(
      conversationTitle(summary({ last_event_preview: "what's 2+2?" })),
    ).toBe("what's 2+2?");
  });

  it("surfaces a backend chat-text preview verbatim", () => {
    expect(
      conversationTitle(summary({ last_event_preview: "User: hello" })),
    ).toBe("User: hello");
  });

  it("trims surrounding whitespace from the preview", () => {
    expect(
      conversationTitle(summary({ last_event_preview: "  hello world  " })),
    ).toBe("hello world");
  });

  it("falls back to the placeholder when the preview is null", () => {
    expect(conversationTitle(summary({ last_event_preview: null }))).toBe(
      "Untitled conversation",
    );
  });

  it("falls back to the placeholder when the preview is empty / whitespace", () => {
    expect(conversationTitle(summary({ last_event_preview: "" }))).toBe(
      "Untitled conversation",
    );
    expect(conversationTitle(summary({ last_event_preview: "   " }))).toBe(
      "Untitled conversation",
    );
  });
});
