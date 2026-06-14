import { describe, it, expect } from "vitest";
import type {
  Conversation,
  IssueSummaryRecord,
  SessionSummaryRecord,
} from "@hydra/api";
import { resolveSessionDisplay } from "../sessionDisplay";
import { makeStatusDef } from "../../../test-utils/statusDef";

function session(overrides: Partial<SessionSummaryRecord["session"]> = {}): SessionSummaryRecord {
  return {
    session_id: "t-1",
    version: 1n,
    timestamp: "2026-03-15T10:00:00.000Z",
    session: {
      prompt: "You are an agent. Task: do the thing.",
      creator: "alice",
      status: "running",
      ...overrides,
    },
  };
}

function issueRecord(id: string, title: string, assignee?: string | null): IssueSummaryRecord {
  return {
    issue_id: id,
    version: 1n,
    timestamp: "2026-03-15T10:00:00.000Z",
    creation_time: "2026-03-15T10:00:00.000Z",
    issue: {
      type: "task",
      title,
      description: "",
      creator: "alice",
      status: makeStatusDef("in-progress"),
      project_id: "j-defaul",
      // Phase 4b: tests pass a bare username; wrap as `Principal::Agent`
      // since sessions historically used the assignee as the agent name.
      assignee: assignee ? { Agent: { name: assignee } } : null,
      dependencies: [],
      patches: [],
    },
  };
}

function conversation(
  id: string,
  title: string | null,
  agentName: string | null,
): Conversation {
  return {
    conversation_id: id,
    title,
    agent_name: agentName,
    status: "active",
    creator: "alice",
    created_at: "2026-03-15T10:00:00.000Z",
    updated_at: "2026-03-15T10:00:00.000Z",
  };
}

describe("resolveSessionDisplay", () => {
  it("prefers linked issue title over conversation title and prompt", () => {
    const rec = session({
      spawned_from: "i-1",
      conversation_id: "c-1",
    });
    const issueMap = new Map([["i-1", issueRecord("i-1", "Migrate OAuth", "swe")]]);
    const convMap = new Map([["c-1", conversation("c-1", "Auth sync", "scribe")]]);
    const display = resolveSessionDisplay(rec, issueMap, convMap);
    expect(display.title).toBe("Migrate OAuth");
    expect(display.agentName).toBe("swe");
    expect(display.issueId).toBe("i-1");
    expect(display.conversationId).toBe("c-1");
  });

  it("falls back to conversation title when issue has no title", () => {
    const rec = session({ conversation_id: "c-1" });
    const convMap = new Map([["c-1", conversation("c-1", "Auth sync", "scribe")]]);
    const display = resolveSessionDisplay(rec, new Map(), convMap);
    expect(display.title).toBe("Auth sync");
    expect(display.agentName).toBe("scribe");
  });

  it("falls back to prompt snippet when no linked entity is available", () => {
    const rec = session();
    const display = resolveSessionDisplay(rec, new Map(), new Map());
    expect(display.title).toContain("You are an agent");
    expect(display.agentName).toBeNull();
  });

  it("uses conversation agent_name when linked issue has no assignee", () => {
    const rec = session({ spawned_from: "i-1", conversation_id: "c-1" });
    const issueMap = new Map([["i-1", issueRecord("i-1", "Some task", null)]]);
    const convMap = new Map([["c-1", conversation("c-1", "Sync", "scribe")]]);
    const display = resolveSessionDisplay(rec, issueMap, convMap);
    expect(display.agentName).toBe("scribe");
  });
});
