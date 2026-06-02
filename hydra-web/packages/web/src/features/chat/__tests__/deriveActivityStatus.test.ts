import { describe, it, expect } from "vitest";
import type { ConversationStatus, JsonValue, SessionEvent } from "@hydra/api";
import { mergeOptimisticEvents } from "../mergeOptimisticEvents";
import {
  deriveActivityStatus,
  TOOL_DESCRIPTION_MAX_CHARS,
  TOOL_LABELS,
} from "../deriveActivityStatus";

const TS = "2026-05-14T00:00:00Z";

function userMessage(content = "hi"): SessionEvent {
  return { type: "user_message", content, timestamp: TS };
}
function assistantMessage(content = "ok"): SessionEvent {
  return { type: "assistant_message", content, timestamp: TS };
}
function toolUse(tool_name: string, payload: JsonValue = null): SessionEvent {
  return { type: "tool_use", tool_name, payload, timestamp: TS };
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

  // ── Tool-call description branch (issue [[i-jxamlakh]]) ───────────────
  // Tools whose payload carries a human-readable `description` field
  // (e.g. Bash, Agent) surface that string instead of the generic label.

  it("ToolUse{Bash} with payload.description → renders the description verbatim", () => {
    expect(
      deriveActivityStatus(
        [toolUse("Bash", { command: "ls -la", description: "List files in repo root" })],
        open,
      ),
    ).toEqual({ text: "List files in repo root" });
  });

  it("ToolUse{Agent} with payload.description → renders the description, no toolName", () => {
    const result = deriveActivityStatus(
      [toolUse("Agent", { description: "Search for OAuth handler usages" })],
      open,
    );
    expect(result).toEqual({ text: "Search for OAuth handler usages" });
    expect(result?.toolName).toBeUndefined();
  });

  it("ToolUse with payload.description overrides the TOOL_LABELS entry for that tool", () => {
    // Bash maps to 'Running command' via TOOL_LABELS; description wins.
    expect(
      deriveActivityStatus(
        [toolUse("Bash", { description: "Run the test suite" })],
        open,
      ),
    ).toEqual({ text: "Run the test suite" });
  });

  it("ToolUse with payload.description for an unknown tool → description, no 'Using <tool>' fallback", () => {
    expect(
      deriveActivityStatus(
        [toolUse("MysteryTool", { description: "Do the thing" })],
        open,
      ),
    ).toEqual({ text: "Do the thing" });
  });

  it("ToolUse with non-string payload.description falls through to the existing mapping", () => {
    // numeric description → ignored; Grep label still wins.
    expect(
      deriveActivityStatus([toolUse("Grep", { description: 42 })], open),
    ).toEqual({ text: "Searching code" });
  });

  it("ToolUse with empty / whitespace-only description falls through to the existing mapping", () => {
    expect(
      deriveActivityStatus([toolUse("Bash", { description: "   " })], open),
    ).toEqual({ text: "Running command" });
    expect(
      deriveActivityStatus([toolUse("Bash", { description: "" })], open),
    ).toEqual({ text: "Running command" });
  });

  it("ToolUse with array payload (no description field) falls through", () => {
    expect(deriveActivityStatus([toolUse("Bash", ["ls", "-la"])], open)).toEqual({
      text: "Running command",
    });
  });

  it("ToolUse with null payload falls through (back-compat with legacy events)", () => {
    expect(deriveActivityStatus([toolUse("Bash", null)], open)).toEqual({
      text: "Running command",
    });
  });

  it("trims surrounding whitespace before rendering the description", () => {
    expect(
      deriveActivityStatus(
        [toolUse("Bash", { description: "  Build the project  " })],
        open,
      ),
    ).toEqual({ text: "Build the project" });
  });

  it("uses only the first line of a multi-line description", () => {
    expect(
      deriveActivityStatus(
        [
          toolUse("Bash", {
            description: "Run the tests\n(then push)",
          }),
        ],
        open,
      ),
    ).toEqual({ text: "Run the tests" });
  });

  it("truncates long descriptions with an ellipsis", () => {
    const long = "A".repeat(TOOL_DESCRIPTION_MAX_CHARS + 50);
    const result = deriveActivityStatus(
      [toolUse("Bash", { description: long })],
      open,
    );
    expect(result?.text.length).toBe(TOOL_DESCRIPTION_MAX_CHARS + 1); // +1 for the ellipsis char
    expect(result?.text.endsWith("…")).toBe(true);
    expect(result?.text.startsWith("A".repeat(TOOL_DESCRIPTION_MAX_CHARS))).toBe(true);
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
