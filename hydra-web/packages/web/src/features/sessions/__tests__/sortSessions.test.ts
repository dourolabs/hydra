import { describe, it, expect } from "vitest";
import type { SessionSummaryRecord, Status } from "@hydra/api";
import { sortSessionsActiveFirst } from "../sortSessions";

function makeRecord(
  id: string,
  status: Status,
  opts: {
    creationTime?: string;
    startTime?: string;
    endTime?: string;
    timestamp?: string;
  } = {},
): SessionSummaryRecord {
  return {
    session_id: id,
    version: 1n,
    timestamp: opts.timestamp ?? "2026-01-01T00:00:00.000Z",
    session: {
      prompt: `prompt for ${id}`,
      creator: "swe",
      status,
      creation_time: opts.creationTime,
      start_time: opts.startTime,
      end_time: opts.endTime,
    },
  };
}

describe("sortSessionsActiveFirst", () => {
  it("places active sessions before terminal sessions", () => {
    const records = [
      makeRecord("done-1", "complete", { endTime: "2026-04-01T00:00:00.000Z" }),
      makeRecord("run-1", "running", { startTime: "2026-03-01T00:00:00.000Z" }),
      makeRecord("fail-1", "failed", { endTime: "2026-04-05T00:00:00.000Z" }),
      makeRecord("pend-1", "pending", { creationTime: "2026-03-02T00:00:00.000Z" }),
      makeRecord("cre-1", "created", { creationTime: "2026-03-03T00:00:00.000Z" }),
    ];

    const sorted = sortSessionsActiveFirst(records);

    expect(sorted.slice(0, 3).map((r) => r.session_id).sort()).toEqual([
      "cre-1",
      "pend-1",
      "run-1",
    ]);
    expect(sorted.slice(3).map((r) => r.session_id)).toEqual(["fail-1", "done-1"]);
  });

  it("orders active sessions by most recent start/creation time desc", () => {
    const records = [
      makeRecord("a", "running", { startTime: "2026-03-01T00:00:00.000Z" }),
      makeRecord("b", "running", { startTime: "2026-03-03T00:00:00.000Z" }),
      makeRecord("c", "pending", { creationTime: "2026-03-02T00:00:00.000Z" }),
    ];

    const sorted = sortSessionsActiveFirst(records);

    expect(sorted.map((r) => r.session_id)).toEqual(["b", "c", "a"]);
  });

  it("orders terminal sessions by end_time desc", () => {
    const records = [
      makeRecord("old", "complete", { endTime: "2026-02-01T00:00:00.000Z" }),
      makeRecord("mid", "failed", { endTime: "2026-03-01T00:00:00.000Z" }),
      makeRecord("new", "complete", { endTime: "2026-04-01T00:00:00.000Z" }),
    ];

    const sorted = sortSessionsActiveFirst(records);

    expect(sorted.map((r) => r.session_id)).toEqual(["new", "mid", "old"]);
  });

  it("falls back to record timestamp when end_time is missing", () => {
    const records = [
      makeRecord("with-end", "complete", { endTime: "2026-03-01T00:00:00.000Z" }),
      makeRecord("only-ts", "failed", { timestamp: "2026-05-01T00:00:00.000Z" }),
    ];

    const sorted = sortSessionsActiveFirst(records);

    expect(sorted.map((r) => r.session_id)).toEqual(["only-ts", "with-end"]);
  });

  it("returns a new array without mutating the input", () => {
    const records: SessionSummaryRecord[] = [
      makeRecord("a", "complete", { endTime: "2026-01-01T00:00:00.000Z" }),
      makeRecord("b", "running", { startTime: "2026-01-02T00:00:00.000Z" }),
    ];
    const original = [...records];

    const sorted = sortSessionsActiveFirst(records);

    expect(records).toEqual(original);
    expect(sorted).not.toBe(records);
  });

  it("handles an empty list", () => {
    expect(sortSessionsActiveFirst([])).toEqual([]);
  });
});
