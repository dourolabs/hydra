import type { ConversationStatus, SessionEvent } from "@hydra/api";

export interface ActivityStatus {
  /**
   * Human-readable label for the indicator. Renderers display this as-is and
   * append `toolName` (if present) as inline code — gives the renderer the
   * raw tool name without forcing it to parse markdown out of `text`.
   */
  text: string;
  /**
   * Only populated for the fallback `Using <tool_name>` branch; the renderer
   * should append it after `text` styled as inline code.
   */
  toolName?: string;
}

/**
 * Tool-name → human label table. Kept as a plain record so it's trivial to
 * extend as Claude grows new tool names without touching the derivation logic.
 * Tool-name matching is case-sensitive against the names emitted by Claude.
 */
export const TOOL_LABELS: Record<string, string> = {
  Bash: "Running command",
  Read: "Reading file",
  Edit: "Editing file",
  Write: "Editing file",
  NotebookEdit: "Editing file",
  Grep: "Searching code",
  Glob: "Searching files",
  WebFetch: "Fetching from web",
  WebSearch: "Searching the web",
  Task: "Delegating subtask",
  Agent: "Delegating subtask",
  TodoWrite: "Updating plan",
};

/**
 * Derive what the transient activity indicator should display based on the
 * tail of the (already merged with optimistic) `SessionEvent` log and the
 * conversation's status. Returns `null` to mean "hide the indicator".
 *
 * The function is intentionally pure — no hooks, no React imports — so it
 * can be unit-tested table-style and consumed from any render surface.
 *
 * Mapping (see issue [[i-hgmaqhor]]):
 * - `UserMessage` tail → `Thinking…`
 * - `ToolUse` tail → tool-specific label from `TOOL_LABELS`, fallback to
 *   `Using <tool_name>` (toolName exposed separately for inline-code render).
 * - `Resumed` tail → `Resuming session…`
 * - `AssistantMessage` / `Suspending` / `Closed` / `Unknown` tail → `null`
 *   (the next assistant message is the visible signal; system-event tails
 *   don't need a live indicator).
 * - Empty events or `Closed` conversation → `null`.
 */
export function deriveActivityStatus(
  events: readonly SessionEvent[],
  conversationStatus: ConversationStatus,
): ActivityStatus | null {
  if (conversationStatus === "closed") return null;
  if (events.length === 0) return null;

  const tail = events[events.length - 1];
  switch (tail.type) {
    case "user_message":
      return { text: "Thinking…" };
    case "tool_use": {
      const label = TOOL_LABELS[tail.tool_name];
      if (label) return { text: label };
      return { text: "Using", toolName: tail.tool_name };
    }
    case "resumed":
      return { text: "Resuming session…" };
    case "assistant_message":
    case "suspending":
    case "closed":
    case "unknown":
      return null;
  }
}
