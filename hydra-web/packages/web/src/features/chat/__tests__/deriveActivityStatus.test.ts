import { describe, it, expect } from "vitest";
import type { ConversationStatus, SessionEvent } from "@hydra/api";
import { mergeOptimisticEvents } from "../mergeOptimisticEvents";
import {
  deriveActivityStatus,
  TOOL_LABELS,
} from "../deriveActivityStatus";

const TS = "2026-05-14T00:00:00Z";

function userMessage(content = "hi"): SessionEvent {
  return { type: "user_message", content, timestamp: TS };
}
function assistantMessage(content = "ok"): SessionEvent {
  return { type: "assistant_message", content, timestamp: TS };
}
function toolUse(tool_name: string): SessionEvent {
  return { type: "tool_use", tool_name, payload: null, timestamp: TS };
}
function resumed(): SessionEvent {
  return {
    type: "resumed",
    from_session_id: "t-prev0001",
    source: "native",
    timestamp: TS,
  };
}
function suspending(): SessionEvent {
  return { type: "suspending", reason: "manual", timestamp: TS };
}
function closed(): SessionEvent {
  return { type: "closed", timestamp: TS };
}
function unknown(): SessionEvent {
  return { type: "unknown" };
}

describe("deriveActivityStatus — tail mapping", () => {
  const open: ConversationStatus = "active";

  it("UserMessage tail → 'Thinking…'", () => {
    expect(deriveActivityStatus([userMessage()], open)).toEqual({
      text: "Thinking…",
    });
  });

  // One row per known tool from the issue mapping table.
  const knownTools: Array<[string, string]> = [
    ["Bash", "Running command"],
    ["Read", "Reading file"],
    ["Edit", "Editing file"],
    ["Write", "Editing file"],
    ["NotebookEdit", "Editing file"],
    ["Grep", "Searching code"],
    ["Glob", "Searching files"],
    ["WebFetch", "Fetching from web"],
    ["WebSearch", "Searching the web"],
    ["Task", "Delegating subtask"],
    ["Agent", "Delegating subtask"],
    ["TodoWrite", "Updating plan"],
  ];
  for (const [tool, label] of knownTools) {
    it(`ToolUse{${tool}} tail → '${label}'`, () => {
      expect(deriveActivityStatus([toolUse(tool)], open)).toEqual({
        text: label,
      });
    });
  }

  it("ToolUse with unknown tool → 'Using' + toolName for inline-code render", () => {
    expect(deriveActivityStatus([toolUse("MysteryTool")], open)).toEqual({
      text: "Using",
      toolName: "MysteryTool",
    });
  });

  it("Resumed tail → 'Resuming session…'", () => {
    expect(deriveActivityStatus([resumed()], open)).toEqual({
      text: "Resuming session…",
    });
  });

  it("AssistantMessage tail → null (next message is the visible signal)", () => {
    expect(deriveActivityStatus([userMessage(), assistantMessage()], open)).toBeNull();
  });

  it("Suspending tail → null", () => {
    expect(deriveActivityStatus([suspending()], open)).toBeNull();
  });

  it("Closed event tail → null", () => {
    expect(deriveActivityStatus([closed()], open)).toBeNull();
  });

  it("Unknown tail → null", () => {
    expect(deriveActivityStatus([unknown()], open)).toBeNull();
  });

  it("empty events → null", () => {
    expect(deriveActivityStatus([], open)).toBeNull();
  });

  it("conversation status 'closed' → null regardless of tail", () => {
    expect(deriveActivityStatus([userMessage()], "closed")).toBeNull();
    expect(deriveActivityStatus([toolUse("Bash")], "closed")).toBeNull();
  });

  it("status 'idle' is treated like any non-closed status", () => {
    expect(deriveActivityStatus([userMessage()], "idle")).toEqual({
      text: "Thinking…",
    });
  });

  it("inspects only the last event, not earlier ones", () => {
    // assistant tail despite an earlier user_message → hidden.
    expect(
      deriveActivityStatus([userMessage(), assistantMessage()], "active"),
    ).toBeNull();
    // tool_use tail wins over a stale user_message earlier in the log.
    expect(
      deriveActivityStatus([userMessage(), toolUse("Bash")], "active"),
    ).toEqual({ text: "Running command" });
  });

  it("optimistic user_message merge result yields 'Thinking…'", () => {
    // Transcript hasn't seen the message yet; optimistic buffer carries it.
    const merged = mergeOptimisticEvents([], [userMessage("hello there")]);
    expect(deriveActivityStatus(merged, "active")).toEqual({
      text: "Thinking…",
    });
  });
});

describe("TOOL_LABELS table", () => {
  it("exports as a plain Record<string, string> for extensibility", () => {
    expect(typeof TOOL_LABELS).toBe("object");
    expect(TOOL_LABELS.Bash).toBe("Running command");
  });
});
