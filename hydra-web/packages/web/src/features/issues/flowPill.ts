import type { StatusDefinition } from "@hydra/api";
import type { FlowPillPhase } from "@hydra/ui";

/** One direct neighbour of an issue: a direct child or a direct blocker. */
export interface NeighborStatus {
  id: string;
  status: StatusDefinition;
}

/** Local neighborhood for one issue: its direct blockers and direct children. */
export interface IssueNeighborhood {
  blockers: NeighborStatus[];
  children: NeighborStatus[];
}

export interface FlowPillState {
  phase: FlowPillPhase;
  num: number;
  den: number;
  title: string;
}

/**
 * Compute the FlowPill state from an issue's direct blockers and direct
 * children. Returns `null` when no pill should render (no blockers and no
 * children).
 *
 * Logic (per design):
 * - any active blocker (`unblocks_dependents == false`) → "blocked", num =
 *   active blockers, den = total blockers.
 * - no blockers, no children → render nothing.
 * - no active blockers, children, not all completed → "progress", num =
 *   completed children (`unblocks_parents == true`), den = total children.
 * - no active blockers, children, all completed → "done", num = den.
 */
export function computeFlowPillState(
  neighborhood: IssueNeighborhood | undefined,
): FlowPillState | null {
  const blockers = neighborhood?.blockers ?? [];
  const children = neighborhood?.children ?? [];

  if (blockers.length > 0) {
    const active = blockers.filter((b) => !b.status.unblocks_dependents).length;
    if (active > 0) {
      return {
        phase: "blocked",
        num: active,
        den: blockers.length,
        title: `${active} of ${blockers.length} blockers active`,
      };
    }
  }

  if (children.length === 0) return null;

  const done = children.filter((c) => c.status.unblocks_parents).length;
  if (done >= children.length) {
    return {
      phase: "done",
      num: children.length,
      den: children.length,
      title: `${children.length} of ${children.length} children completed`,
    };
  }

  return {
    phase: "progress",
    num: done,
    den: children.length,
    title: `${done} of ${children.length} children completed`,
  };
}
