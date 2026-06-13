import { describe, it, expect } from "vitest";
import type {
  IssueSummaryRecord,
  ProjectRecord,
  StatusDefinition,
} from "@hydra/api";
import { buildSections, UNRESOLVED_GROUP_KEY } from "../projectSections";
import { makeStatusDef } from "../../../../test-utils/statusDef";

function makeStatus(key: string): StatusDefinition {
  return makeStatusDef(key);
}

function makeProject(
  id: string,
  key: string,
  name: string = key,
  statuses: StatusDefinition[] = [makeStatus("open")],
  priority: number = 0,
): ProjectRecord {
  return {
    project_id: id,
    version: 1,
    project: {
      key,
      name,
      statuses,
      creator: "alice",
      archived: false,
      priority,
    },
  };
}

function makeIssue(
  id: string,
  projectId: string,
  statusKey: string = "open",
): IssueSummaryRecord {
  return {
    issue_id: id,
    version: 1n,
    timestamp: "2026-06-01T00:00:00Z",
    creation_time: "2026-06-01T00:00:00Z",
    issue: {
      type: "task",
      title: id,
      description: "",
      creator: "alice",
      status: makeStatus(statusKey),
      project_id: projectId,
      assignee: null,
      progress: "",
      dependencies: [],
      patches: [],
      labels: [],
    },
  } as unknown as IssueSummaryRecord;
}

describe("buildSections", () => {
  it("returns flat: true with no sections when projects are undefined", () => {
    const issues = [makeIssue("i-a", "j-defaul")];
    const { sections, flat } = buildSections(issues, undefined);
    expect(flat).toBe(true);
    expect(sections).toEqual([]);
  });

  it("returns flat: true with no sections when projects are empty", () => {
    const issues = [makeIssue("i-a", "j-defaul")];
    const { sections, flat } = buildSections(issues, []);
    expect(flat).toBe(true);
    expect(sections).toEqual([]);
  });

  it("emits sections in the server-stream first-occurrence order", () => {
    // The list endpoint emits issues already sorted by project priority,
    // so first-occurrence iteration here yields project sections in
    // priority order — even when the projects list isn't `default`-first.
    const projects = [
      makeProject("j-defaul", "default"),
      makeProject("j-alpha", "alpha"),
    ];
    // Issues arrive with `alpha` first, then `default`. buildSections must
    // preserve that order — no default-first reshuffle.
    const issues = [
      makeIssue("i-1", "j-alpha"),
      makeIssue("i-2", "j-alpha"),
      makeIssue("i-3", "j-defaul"),
    ];
    const { sections, flat } = buildSections(issues, projects);
    expect(flat).toBe(false);
    expect(sections.map((s) => s.projectKey)).toEqual(["alpha", "default"]);
    expect(sections[0].issues.map((i) => i.issue_id)).toEqual(["i-1", "i-2"]);
    expect(sections[1].issues.map((i) => i.issue_id)).toEqual(["i-3"]);
  });

  it("preserves the server-supplied issue order within each section (no re-sort)", () => {
    const projects = [makeProject("j-defaul", "default")];
    const issues = [
      makeIssue("i-c", "j-defaul"),
      makeIssue("i-a", "j-defaul"),
      makeIssue("i-b", "j-defaul"),
    ];
    const { sections } = buildSections(issues, projects);
    expect(sections).toHaveLength(1);
    expect(sections[0].issues.map((i) => i.issue_id)).toEqual([
      "i-c",
      "i-a",
      "i-b",
    ]);
  });

  it("skips projects with no loaded issues", () => {
    const projects = [
      makeProject("j-defaul", "default"),
      makeProject("j-alpha", "alpha"),
    ];
    const issues = [makeIssue("i-1", "j-alpha")];
    const { sections } = buildSections(issues, projects);
    expect(sections.map((s) => s.projectKey)).toEqual(["alpha"]);
  });

  it("falls through to an orphan section for unknown project_ids", () => {
    const projects = [makeProject("j-defaul", "default")];
    const issues = [
      makeIssue("i-1", "j-defaul"),
      makeIssue("i-orphan", "j-removed"),
    ];
    const { sections } = buildSections(issues, projects);
    expect(sections).toHaveLength(2);
    expect(sections[0].projectKey).toBe("default");
    expect(sections[1].groupKey).toBe("j-removed");
    expect(sections[1].projectName).toBeNull();
    expect(sections[1].statuses).toEqual([]);
    expect(sections[1].issues.map((i) => i.issue_id)).toEqual(["i-orphan"]);
  });

  it("renders the `UNRESOLVED_GROUP_KEY` sentinel as projectKey 'unknown'", () => {
    const projects = [makeProject("j-defaul", "default")];
    // Synthetic case: an issue carrying the sentinel as its project_id.
    const issues = [makeIssue("i-orphan", UNRESOLVED_GROUP_KEY)];
    const { sections } = buildSections(issues, projects);
    expect(sections).toHaveLength(1);
    expect(sections[0].groupKey).toBe(UNRESOLVED_GROUP_KEY);
    expect(sections[0].projectKey).toBe("unknown");
  });
});
