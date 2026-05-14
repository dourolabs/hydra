import { describe, it, expect } from "vitest";
import type { SessionSummaryRecord } from "@hydra/api";
import { sortSessions } from "../sortSessions";

function rec(
  id: string,
  status: SessionSummaryRecord["session"]["status"],
  overrides: Partial<SessionSummaryRecord["session"]> = {},
  timestamp = "2026-01-01T00:00:00.000Z",
): SessionSummaryRecord {
  return {
    session_id: id,
    version: 1n,
    timestamp,
    session: {
      prompt: `prompt for ${id}`,
      creator: "swe",
      status,
      ...overrides,
    },
  };
}

describe("sortSessions", () => {
  it("places active sessions before terminal sessions", () => {
    const sessions: SessionSummaryRecord[] = [
      rec("t-complete", "complete", { end_time: "2026-03-15T12:00:00Z" }),
      rec("t-running", "running", { start_time: "2026-03-15T11:00:00Z" }),
      rec("t-failed", "failed", { end_time: "2026-03-15T13:00:00Z" }),
      rec("t-created", "created", { creation_time: "2026-03-15T10:00:00Z" }),
      rec("t-pending", "pending", { creation_time: "2026-03-15T09:30:00Z" }),
    ];

    const sorted = sortSessions(sessions);
    const ids = sorted.map((s) => s.session_id);

    // First three must be the active ones.
    expect(ids.slice(0, 3)).toEqual(
      expect.arrayContaining(["t-running", "t-created", "t-pending"]),
    );
    expect(["t-running", "t-created", "t-pending"]).toContain(ids[0]);
    expect(["t-running", "t-created", "t-pending"]).toContain(ids[1]);
    expect(["t-running", "t-created", "t-pending"]).toContain(ids[2]);
    // Last two must be the terminal ones.
    expect(ids.slice(3)).toEqual(["t-failed", "t-complete"]);
  });

  it("orders active sessions by start_time desc, falling back to creation_time", () => {
    const sessions: SessionSummaryRecord[] = [
      rec("a-old", "running", { start_time: "2026-03-15T09:00:00Z" }),
      rec("a-new", "running", { start_time: "2026-03-15T15:00:00Z" }),
      rec("a-created", "created", { creation_time: "2026-03-15T10:00:00Z" }),
    ];

    const sorted = sortSessions(sessions).map((s) => s.session_id);
    expect(sorted).toEqual(["a-new", "a-created", "a-old"]);
  });

  it("orders terminal sessions by end_time desc, falling back to timestamp", () => {
    const sessions: SessionSummaryRecord[] = [
      rec(
        "no-end",
        "failed",
        {},
        "2026-03-15T14:00:00.000Z",
      ),
      rec("late", "complete", { end_time: "2026-03-15T13:00:00Z" }),
      rec("early", "complete", { end_time: "2026-03-15T10:00:00Z" }),
    ];

    const sorted = sortSessions(sessions).map((s) => s.session_id);
    // no-end has timestamp 14:00; late has end_time 13:00; early has end_time 10:00.
    expect(sorted).toEqual(["no-end", "late", "early"]);
  });

  it("returns an empty array for an empty input", () => {
    expect(sortSessions([])).toEqual([]);
  });
});
