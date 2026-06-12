import { describe, it, expect } from "vitest";
import type { ConversationStatus, JsonValue, SessionEvent } from "@hydra/api";
import { mergeOptimisticEvents } from "../mergeOptimisticEvents";
import {
  categorizeTool,
  deriveActivitySteps,
  TOOL_DESCRIPTION_MAX_CHARS,
  TOOL_LABELS,
} from "../deriveActivitySteps";

const T0 = "2026-05-14T00:00:00Z";
const T1 = "2026-05-14T00:00:01Z";
const T2 = "2026-05-14T00:00:02Z";
const T3 = "2026-05-14T00:00:03Z";
const T4 = "2026-05-14T00:00:04Z";
const N0 = Date.parse(T0);
const N1 = Date.parse(T1);
const N2 = Date.parse(T2);
const N3 = Date.parse(T3);
const N4 = Date.parse(T4);

// Deterministic `now` for tests that exercise terminal-state closeout.
const NOW = Date.parse("2026-05-14T00:00:10Z");

function userMessage(ts = T0, content = "hi"): SessionEvent {
  return { type: "user_message", content, timestamp: ts };
}
function assistantMessage(ts = T0, content = "ok"): SessionEvent {
  return { type: "assistant_message", content, timestamp: ts };
}
function toolUse(
  tool_name: string,
  payload: JsonValue = null,
  ts: string = T0,
): SessionEvent {
  return { type: "tool_use", tool_name, payload, timestamp: ts };
}
function resumed(ts = T0): SessionEvent {
  return {
    type: "resumed",
    from_session_id: "t-prev0001",
    source: "native",
    timestamp: ts,
  };
}
function suspending(ts = T0): SessionEvent {
  return { type: "suspending", reason: "manual", timestamp: ts };
}
function closedEv(ts = T0): SessionEvent {
  return { type: "closed", timestamp: ts };
}
function unknownEv(): SessionEvent {
  return { type: "unknown" };
}
function systemEvent(ts = T0, childId = "i-child01"): SessionEvent {
  return {
    type: "system_event",
    kind: { kind: "child_unblocked", child_id: childId, new_status: "in-progress" },
    timestamp: ts,
  };
}

describe("deriveActivitySteps — tail mapping (display compat)", () => {
  const open: ConversationStatus = "active";

  it("UserMessage tail → 'Thinking…' current step (think category)", () => {
    const run = deriveActivitySteps([userMessage(T0)], open, NOW);
    expect(run.current).toMatchObject({
      category: "think",
      verb: "Thinking…",
      detail: null,
      toolName: null,
      startTs: N0,
      endTs: null,
    });
    expect(run.state).toBe("live");
  });

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
    it(`ToolUse{${tool}} tail → '${label}' current step`, () => {
      const run = deriveActivitySteps([toolUse(tool)], open, NOW);
      expect(run.current).toMatchObject({
        verb: label,
        detail: null,
        toolName: null,
      });
    });
  }

  it("ToolUse with unknown tool → 'Using' + toolName on the step", () => {
    const run = deriveActivitySteps([toolUse("MysteryTool")], open, NOW);
    expect(run.current).toMatchObject({
      category: "run",
      verb: "Using",
      detail: null,
      toolName: "MysteryTool",
    });
  });

  it("ToolUse{Bash} with payload.description → detail wins, toolName null", () => {
    const run = deriveActivitySteps(
      [toolUse("Bash", { command: "ls -la", description: "List files in repo root" })],
      open,
      NOW,
    );
    expect(run.current).toMatchObject({
      verb: "Running command",
      detail: "List files in repo root",
      toolName: null,
    });
  });

  it("ToolUse{Agent} with payload.description → detail wins, no toolName", () => {
    const run = deriveActivitySteps(
      [toolUse("Agent", { description: "Search for OAuth handler usages" })],
      open,
      NOW,
    );
    expect(run.current?.detail).toBe("Search for OAuth handler usages");
    expect(run.current?.toolName).toBeNull();
  });

  it("ToolUse with payload.description for an unknown tool → detail, no toolName", () => {
    const run = deriveActivitySteps(
      [toolUse("MysteryTool", { description: "Do the thing" })],
      open,
      NOW,
    );
    expect(run.current?.detail).toBe("Do the thing");
    expect(run.current?.toolName).toBeNull();
  });

  it("ToolUse with non-string payload.description falls through to label", () => {
    const run = deriveActivitySteps(
      [toolUse("Grep", { description: 42 })],
      open,
      NOW,
    );
    expect(run.current?.verb).toBe("Searching code");
    expect(run.current?.detail).toBeNull();
  });

  it("ToolUse with empty / whitespace-only description falls through to label", () => {
    expect(
      deriveActivitySteps([toolUse("Bash", { description: "   " })], open, NOW)
        .current?.verb,
    ).toBe("Running command");
    expect(
      deriveActivitySteps([toolUse("Bash", { description: "" })], open, NOW)
        .current?.verb,
    ).toBe("Running command");
  });

  it("ToolUse with array payload (no description field) falls through", () => {
    const run = deriveActivitySteps(
      [toolUse("Bash", ["ls", "-la"])],
      open,
      NOW,
    );
    expect(run.current?.verb).toBe("Running command");
    expect(run.current?.detail).toBeNull();
  });

  it("ToolUse with null payload falls through (back-compat with legacy events)", () => {
    const run = deriveActivitySteps([toolUse("Bash", null)], open, NOW);
    expect(run.current?.verb).toBe("Running command");
    expect(run.current?.detail).toBeNull();
  });

  it("trims surrounding whitespace before storing the description", () => {
    const run = deriveActivitySteps(
      [toolUse("Bash", { description: "  Build the project  " })],
      open,
      NOW,
    );
    expect(run.current?.detail).toBe("Build the project");
  });

  it("uses only the first line of a multi-line description", () => {
    const run = deriveActivitySteps(
      [toolUse("Bash", { description: "Run the tests\n(then push)" })],
      open,
      NOW,
    );
    expect(run.current?.detail).toBe("Run the tests");
  });

  it("truncates long descriptions with an ellipsis", () => {
    const long = "A".repeat(TOOL_DESCRIPTION_MAX_CHARS + 50);
    const run = deriveActivitySteps(
      [toolUse("Bash", { description: long })],
      open,
      NOW,
    );
    const detail = run.current?.detail ?? "";
    expect(detail.length).toBe(TOOL_DESCRIPTION_MAX_CHARS + 1);
    expect(detail.endsWith("…")).toBe(true);
    expect(detail.startsWith("A".repeat(TOOL_DESCRIPTION_MAX_CHARS))).toBe(true);
  });

  it("SystemEvent tail → 'Thinking…' current step (think category)", () => {
    const run = deriveActivitySteps([systemEvent(T0)], open, NOW);
    expect(run.current).toMatchObject({
      category: "think",
      verb: "Thinking…",
      detail: null,
      toolName: null,
      startTs: N0,
      endTs: null,
    });
    expect(run.state).toBe("live");
  });

  it("SystemEvent mid-history does not dirty steps accumulator", () => {
    const run = deriveActivitySteps(
      [
        userMessage(T0),
        toolUse("Bash", null, T1),
        systemEvent(T2),
        toolUse("Read", null, T3),
      ],
      open,
      NOW,
    );
    // Only the two tool_use events accumulate as steps; the SystemEvent must
    // not surface as a phantom step.
    expect(run.steps).toHaveLength(2);
    expect(run.steps[0]).toMatchObject({ verb: "Running command", startTs: N1, endTs: N2 });
    expect(run.steps[1]).toMatchObject({ verb: "Reading file", startTs: N3, endTs: null });
    expect(run.current).toBe(run.steps[1]);
  });

  it("Resumed tail → 'Resuming session…' current step", () => {
    const run = deriveActivitySteps([resumed(T0)], open, NOW);
    expect(run.current).toMatchObject({
      category: "think",
      verb: "Resuming session…",
      detail: null,
      toolName: null,
      startTs: N0,
    });
  });

  it("Suspending tail → null current (state still live)", () => {
    const run = deriveActivitySteps([suspending()], open, NOW);
    expect(run.current).toBeNull();
    expect(run.state).toBe("live");
  });

  it("Unknown tail → null current (state still live)", () => {
    const run = deriveActivitySteps([unknownEv()], open, NOW);
    expect(run.current).toBeNull();
    expect(run.state).toBe("live");
  });

  it("optimistic user_message merge still yields 'Thinking…'", () => {
    const merged = mergeOptimisticEvents([], [userMessage(T0, "hello there")]);
    const run = deriveActivitySteps(merged, "active", NOW);
    expect(run.current?.verb).toBe("Thinking…");
  });
});

describe("deriveActivitySteps — steps array, chaining, and state", () => {
  it("empty events + active conversation → null current, state live, no steps", () => {
    const run = deriveActivitySteps([], "active", NOW);
    expect(run.steps).toEqual([]);
    expect(run.current).toBeNull();
    expect(run.state).toBe("live");
    expect(run.startedAt).toBe(0);
  });

  it("empty events + closed conversation → null current, state done", () => {
    const run = deriveActivitySteps([], "closed", NOW);
    expect(run.steps).toEqual([]);
    expect(run.current).toBeNull();
    expect(run.state).toBe("done");
  });

  it("multiple tool_use events chain endTs to next event's startTs", () => {
    const run = deriveActivitySteps(
      [
        userMessage(T0),
        toolUse("Bash", null, T1),
        toolUse("Read", null, T2),
        toolUse("Edit", null, T3),
      ],
      "active",
      NOW,
    );
    expect(run.steps).toHaveLength(3);
    expect(run.steps[0]).toMatchObject({ verb: "Running command", startTs: N1, endTs: N2 });
    expect(run.steps[1]).toMatchObject({ verb: "Reading file", startTs: N2, endTs: N3 });
    expect(run.steps[2]).toMatchObject({ verb: "Editing file", startTs: N3, endTs: null });
    // The active (last) step is the `current`.
    expect(run.current).toBe(run.steps[2]);
    expect(run.state).toBe("live");
    expect(run.startedAt).toBe(N0);
  });

  it("only counts tool_use events since the LAST user_message", () => {
    const run = deriveActivitySteps(
      [
        userMessage(T0),
        toolUse("Bash", null, T1),
        assistantMessage(T2),
        // A new turn begins — earlier tool_use should be excluded.
        userMessage(T3),
        toolUse("Read", null, T4),
      ],
      "active",
      NOW,
    );
    expect(run.steps).toHaveLength(1);
    expect(run.steps[0]).toMatchObject({ verb: "Reading file", startTs: N4 });
    expect(run.startedAt).toBe(N3);
  });

  it("last step endTs uses the next non-tool event's timestamp (e.g. assistant_message)", () => {
    const run = deriveActivitySteps(
      [userMessage(T0), toolUse("Bash", null, T1), assistantMessage(T2)],
      "active",
      NOW,
    );
    expect(run.state).toBe("done");
    expect(run.current).toBeNull();
    expect(run.steps[0].endTs).toBe(N2);
  });

  it("terminal tail assistant_message → state done, current null, all steps closed", () => {
    const run = deriveActivitySteps(
      [
        userMessage(T0),
        toolUse("Bash", null, T1),
        toolUse("Read", null, T2),
        assistantMessage(T3),
      ],
      "active",
      NOW,
    );
    expect(run.state).toBe("done");
    expect(run.current).toBeNull();
    expect(run.steps[0].endTs).toBe(N2);
    expect(run.steps[1].endTs).toBe(N3);
  });

  it("terminal tail closed → state done, current null, all steps closed", () => {
    const run = deriveActivitySteps(
      [userMessage(T0), toolUse("Bash", null, T1), closedEv(T2)],
      "active",
      NOW,
    );
    expect(run.state).toBe("done");
    expect(run.current).toBeNull();
    expect(run.steps[0].endTs).toBe(N2);
  });

  it("conversation status closed + active tail → state done, last step closed to NOW", () => {
    // No terminal event in the transcript yet, but the conversation is closed.
    const run = deriveActivitySteps(
      [userMessage(T0), toolUse("Bash", null, T1)],
      "closed",
      NOW,
    );
    expect(run.state).toBe("done");
    expect(run.current).toBeNull();
    expect(run.steps[0].endTs).toBe(NOW);
  });

  it("startedAt falls back to the first step's start when no user_message precedes it", () => {
    const run = deriveActivitySteps(
      [toolUse("Bash", null, T1), toolUse("Read", null, T2)],
      "active",
      NOW,
    );
    expect(run.startedAt).toBe(N1);
  });
});

describe("categorizeTool", () => {
  const cases: Array<[string, string]> = [
    ["Read", "read"],
    ["Grep", "read"],
    ["Glob", "read"],
    ["WebFetch", "read"],
    ["WebSearch", "read"],
    ["Edit", "edit"],
    ["Write", "edit"],
    ["NotebookEdit", "edit"],
    ["Bash", "run"],
    ["TodoWrite", "think"],
    ["Task", "submit"],
    ["Agent", "submit"],
  ];
  for (const [tool, category] of cases) {
    it(`maps ${tool} → ${category}`, () => {
      expect(categorizeTool(tool)).toBe(category);
    });
  }

  it("unknown tool falls back to 'run'", () => {
    expect(categorizeTool("MysteryTool")).toBe("run");
    expect(categorizeTool("")).toBe("run");
  });
});

describe("TOOL_LABELS table", () => {
  it("exports as a plain Record<string, string> for extensibility", () => {
    expect(typeof TOOL_LABELS).toBe("object");
    expect(TOOL_LABELS.Bash).toBe("Running command");
  });
});
