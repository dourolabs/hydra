import { describe, it, expect } from "vitest";
import { computeFlowPillState } from "./flowPill";
import { makeStatusDef } from "../../test-utils/statusDef";

function active() {
  return { ...makeStatusDef("open"), unblocks_dependents: false };
}

function unblockingBlocker() {
  return { ...makeStatusDef("closed"), unblocks_dependents: true };
}

function completedChild() {
  return { ...makeStatusDef("closed"), unblocks_parents: true };
}

function pendingChild() {
  return { ...makeStatusDef("open"), unblocks_parents: false };
}

describe("computeFlowPillState", () => {
  it("returns null when there are no blockers and no children", () => {
    expect(computeFlowPillState({ blockers: [], children: [] })).toBeNull();
    expect(computeFlowPillState(undefined)).toBeNull();
  });

  it("returns 'blocked' phase counting active blockers against total", () => {
    const result = computeFlowPillState({
      blockers: [
        { id: "b1", status: active() },
        { id: "b2", status: active() },
        { id: "b3", status: unblockingBlocker() },
      ],
      children: [{ id: "c1", status: pendingChild() }],
    });
    expect(result).toEqual({
      phase: "blocked",
      num: 2,
      den: 3,
      title: "2 of 3 blockers active",
    });
  });

  it("falls through to children when every blocker is unblocking", () => {
    const result = computeFlowPillState({
      blockers: [{ id: "b1", status: unblockingBlocker() }],
      children: [
        { id: "c1", status: completedChild() },
        { id: "c2", status: pendingChild() },
      ],
    });
    expect(result).toEqual({
      phase: "progress",
      num: 1,
      den: 2,
      title: "1 of 2 children completed",
    });
  });

  it("returns 'done' when every child status unblocks the parent", () => {
    const result = computeFlowPillState({
      blockers: [],
      children: [
        { id: "c1", status: completedChild() },
        { id: "c2", status: completedChild() },
      ],
    });
    expect(result).toEqual({
      phase: "done",
      num: 2,
      den: 2,
      title: "2 of 2 children completed",
    });
  });

  it("returns 'progress' with num=0 when no child has unblocked yet", () => {
    const result = computeFlowPillState({
      blockers: [],
      children: [
        { id: "c1", status: pendingChild() },
        { id: "c2", status: pendingChild() },
      ],
    });
    expect(result).toEqual({
      phase: "progress",
      num: 0,
      den: 2,
      title: "0 of 2 children completed",
    });
  });

  it("returns null when there are blockers but they are all unblocking and there are no children", () => {
    const result = computeFlowPillState({
      blockers: [{ id: "b1", status: unblockingBlocker() }],
      children: [],
    });
    expect(result).toBeNull();
  });
});
