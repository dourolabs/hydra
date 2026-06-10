import { describe, it, expect } from "vitest";
import {
  DEFAULT_TIME_RANGE,
  readSlicerState,
  timeWindow,
  writeSlicerState,
} from "../slicerState";

describe("slicerState.readSlicerState", () => {
  it("returns defaults for an empty URL", () => {
    const s = readSlicerState(new URLSearchParams());
    expect(s).toEqual({
      range: DEFAULT_TIME_RANGE,
      projectId: null,
      statusKeys: [],
      repoName: null,
      issueTypes: [],
      assignee: null,
      creator: null,
    });
  });

  it("parses every slicer param", () => {
    const params = new URLSearchParams({
      range: "90d",
      project_id: "j-abc",
      status_keys: "open,in-progress",
      repo_name: "dourolabs/hydra",
      issue_type: "feature",
      assignee: "agents/swe",
      creator: "alice",
    });
    const s = readSlicerState(params);
    expect(s).toEqual({
      range: "90d",
      projectId: "j-abc",
      statusKeys: ["open", "in-progress"],
      repoName: "dourolabs/hydra",
      issueTypes: ["feature"],
      assignee: "agents/swe",
      creator: "alice",
    });
  });

  it("prefers plural issue_types over singular issue_type when both are set", () => {
    const params = new URLSearchParams({
      issue_types: "feature,bug",
      issue_type: "chore",
    });
    const s = readSlicerState(params);
    expect(s.issueTypes).toEqual(["feature", "bug"]);
  });

  it("drops invalid range and issue_types values", () => {
    const params = new URLSearchParams({
      range: "junk",
      issue_types: "nope,feature,bogus",
    });
    const s = readSlicerState(params);
    expect(s.range).toBe(DEFAULT_TIME_RANGE);
    expect(s.issueTypes).toEqual(["feature"]);
  });
});

describe("slicerState.writeSlicerState", () => {
  it("sets and deletes single-value fields based on null/non-null", () => {
    const p = writeSlicerState(new URLSearchParams("project_id=j-old"), {
      projectId: null,
      repoName: "dourolabs/hydra",
    });
    expect(p.has("project_id")).toBe(false);
    expect(p.get("repo_name")).toBe("dourolabs/hydra");
  });

  it("joins list values with commas and drops empty lists", () => {
    const p = writeSlicerState(new URLSearchParams("status_keys=stale"), {
      statusKeys: ["open", "in-progress"],
    });
    expect(p.get("status_keys")).toBe("open,in-progress");
  });

  it("writes singular issue_type when exactly one issue type is selected", () => {
    const set = writeSlicerState(new URLSearchParams(), {
      issueTypes: ["feature"],
    });
    expect(set.get("issue_type")).toBe("feature");
    expect(set.has("issue_types")).toBe(false);
  });

  it("writes plural issue_types when multiple issue types are selected", () => {
    const set = writeSlicerState(new URLSearchParams(), {
      issueTypes: ["feature", "bug"],
    });
    expect(set.get("issue_types")).toBe("feature,bug");
    expect(set.has("issue_type")).toBe(false);
  });

  it("collapsing the selection from many to one swaps issue_types for issue_type", () => {
    const collapsed = writeSlicerState(
      new URLSearchParams("issue_types=feature,bug"),
      { issueTypes: ["feature"] },
    );
    expect(collapsed.get("issue_type")).toBe("feature");
    expect(collapsed.has("issue_types")).toBe(false);
  });

  it("growing the selection from one to many swaps issue_type for issue_types", () => {
    const grown = writeSlicerState(new URLSearchParams("issue_type=feature"), {
      issueTypes: ["feature", "bug"],
    });
    expect(grown.get("issue_types")).toBe("feature,bug");
    expect(grown.has("issue_type")).toBe(false);
  });

  it("clearing the selection drops both issue_type keys", () => {
    const clear = writeSlicerState(
      new URLSearchParams("issue_type=feature&issue_types=feature,bug"),
      { issueTypes: [] },
    );
    expect(clear.has("issue_type")).toBe(false);
    expect(clear.has("issue_types")).toBe(false);
  });

  it("always writes the range key when patched", () => {
    const p = writeSlicerState(new URLSearchParams(), { range: "7d" });
    expect(p.get("range")).toBe("7d");
  });
});

describe("slicerState.timeWindow", () => {
  const now = new Date("2026-06-09T12:00:00.000Z");

  it("computes 7d / 30d / 90d windows relative to `now`", () => {
    expect(timeWindow("7d", now)).toEqual({
      from: "2026-06-02T12:00:00.000Z",
      to: "2026-06-09T12:00:00.000Z",
    });
    expect(timeWindow("30d", now).from).toBe("2026-05-10T12:00:00.000Z");
    expect(timeWindow("90d", now).from).toBe("2026-03-11T12:00:00.000Z");
  });

  it("pegs all-time to a fixed origin", () => {
    expect(timeWindow("all-time", now)).toEqual({
      from: "2020-01-01T00:00:00.000Z",
      to: "2026-06-09T12:00:00.000Z",
    });
  });
});
