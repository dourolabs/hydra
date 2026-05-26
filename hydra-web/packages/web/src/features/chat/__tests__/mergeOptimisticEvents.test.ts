import { describe, it, expect } from "vitest";
import type { SessionEvent } from "@hydra/api";
import { mergeOptimisticEvents } from "../mergeOptimisticEvents";

function userMessage(content: string, timestamp: string): SessionEvent {
  return { type: "user_message", content, timestamp };
}

function assistantMessage(content: string, timestamp: string): SessionEvent {
  return { type: "assistant_message", content, timestamp };
}

describe("mergeOptimisticEvents", () => {
  it("appends an optimistic message that has no server-side counterpart yet", () => {
    // This is the in-flight state right after onMutate: transcript still has
    // the prior turn, optimistic carries the freshly-sent message.
    const transcript: SessionEvent[] = [
      userMessage("hi", "2026-05-23T16:59:00Z"),
      assistantMessage("hello", "2026-05-23T16:59:30Z"),
    ];
    const optimistic: SessionEvent[] = [userMessage("how are you", "2026-05-23T17:00:00Z")];

    const result = mergeOptimisticEvents(transcript, optimistic);

    expect(result.map((e) => "content" in e && e.content)).toEqual([
      "hi",
      "hello",
      "how are you",
    ]);
  });

  it("keeps the optimistic message rendered while invalidation refetch is in flight", () => {
    // Regression: onSettled used to call setOptimisticEvents([]) synchronously
    // alongside fire-and-forget invalidateQueries; the just-sent message
    // briefly disappeared between local clear and refetch landing. The merge
    // must hold the optimistic until the real event arrives in `transcript`.
    const transcriptBeforeRefetch: SessionEvent[] = [
      userMessage("hi", "2026-05-23T16:59:00Z"),
    ];
    const optimistic: SessionEvent[] = [userMessage("how are you", "2026-05-23T17:00:00Z")];

    // Even with the mutation `settled` (onSettled has fired), the new event
    // is not yet in the transcript. The optimistic stays visible.
    const result = mergeOptimisticEvents(transcriptBeforeRefetch, optimistic);
    const contents = result.flatMap((e) => ("content" in e ? [e.content] : []));
    expect(contents).toContain("how are you");
  });

  it("drops the optimistic message exactly once when the real event arrives", () => {
    // After the refetch lands the transcript now contains the real event.
    // The merge must remove the optimistic to avoid showing the user-message
    // twice — a render with both would also flicker.
    const transcriptAfterRefetch: SessionEvent[] = [
      userMessage("hi", "2026-05-23T16:59:00Z"),
      userMessage("how are you", "2026-05-23T17:00:00.123Z"),
    ];
    const optimistic: SessionEvent[] = [
      // Note: the optimistic has a slightly different timestamp than the
      // server-assigned one — content is the matching key, not timestamp.
      userMessage("how are you", "2026-05-23T17:00:00.000Z"),
    ];

    const result = mergeOptimisticEvents(transcriptAfterRefetch, optimistic);
    const userMessages = result.filter((e) => e.type === "user_message");
    expect(userMessages).toHaveLength(2); // not 3 — optimistic dropped
    expect(userMessages.map((e) => "content" in e && e.content)).toEqual(["hi", "how are you"]);
  });

  it("reconciles two duplicate sends of the same text one-for-one", () => {
    // A user can hit enter twice with identical content. Each optimistic
    // entry should match a distinct real event, not all collapse to one.
    const transcript: SessionEvent[] = [userMessage("ping", "2026-05-23T17:00:00Z")];
    const optimistic: SessionEvent[] = [
      userMessage("ping", "2026-05-23T17:00:01Z"),
      userMessage("ping", "2026-05-23T17:00:02Z"),
    ];

    const result = mergeOptimisticEvents(transcript, optimistic);
    // One match consumes one optimistic; the other optimistic stays pending.
    const pings = result.filter((e) => e.type === "user_message");
    expect(pings).toHaveLength(2);
  });

  it("layers a non-user_message optimistic event unconditionally", () => {
    // The dedup pass only matches on user_message content. Other types
    // (none currently produced optimistically, but reserved for future) are
    // appended verbatim.
    const transcript: SessionEvent[] = [];
    const optimistic: SessionEvent[] = [
      assistantMessage("hello", "2026-05-23T17:00:00Z"),
    ];
    const result = mergeOptimisticEvents(transcript, optimistic);
    expect(result).toHaveLength(1);
    expect(result[0].type).toBe("assistant_message");
  });

  it("returns the transcript unchanged when there are no optimistic events", () => {
    const transcript: SessionEvent[] = [
      userMessage("hi", "2026-05-23T17:00:00Z"),
    ];
    const result = mergeOptimisticEvents(transcript, []);
    expect(result).toEqual(transcript);
  });
});
