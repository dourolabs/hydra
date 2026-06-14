import { describe, it, expect, vi, beforeEach } from "vitest";
import { render } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import type { IssueVersionRecord, SystemEventKind } from "@hydra/api";

const useIssueMock = vi.fn<(id: string) => {
  data: IssueVersionRecord | undefined;
  isLoading: boolean;
  isError: boolean;
}>();

vi.mock("../../issues/useIssue", () => ({
  useIssue: (id: string) => useIssueMock(id),
}));

vi.mock("../SystemEventBubble.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("../../projects/StatusChip.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("../../../utils/time", () => ({
  formatTimestamp: (s: string) => s,
  formatRelativeTime: (s: string) => s,
  shortRelativeTime: (s: string) => s,
}));

vi.mock("../../../components/Runtime/Runtime", () => ({
  AgoTime: ({ iso }: { iso: string }) => <span data-testid="ago">{iso}</span>,
}));

const { SystemEventBubble } = await import("../SystemEventBubble");

function makeIssueRecord(opts?: {
  title?: string;
  statusKey?: string;
  statusLabel?: string;
  statusColor?: string;
}): IssueVersionRecord {
  return {
    issue_id: "i-childex",
    version: 1n,
    timestamp: "2026-06-12T00:00:00Z",
    creation_time: "2026-06-12T00:00:00Z",
    issue: {
      type: "task",
      title: opts?.title ?? "Re-index search corpus",
      description: "",
      creator: "swe",
      status: {
        key: opts?.statusKey ?? "closed",
        label: opts?.statusLabel ?? "Closed",
        color: opts?.statusColor ?? "#2ecc71",
        position: 0,
        unblocks_parents: true,
        unblocks_dependents: true,
        cascades_to_children: false,
      },
      project_id: "j-defaul",
      assignee: null,
      dependencies: [],
      patches: [],
    },
  };
}

function renderBubble(kind: SystemEventKind, timestamp = "2026-06-12T13:42:00Z") {
  return render(
    <MemoryRouter>
      <SystemEventBubble kind={kind} timestamp={timestamp} />
    </MemoryRouter>,
  );
}

describe("SystemEventBubble — child_unblocked", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders the resolved child title and StatusChip label when the issue query resolves", () => {
    useIssueMock.mockReturnValue({
      data: makeIssueRecord({ title: "Re-index search corpus", statusLabel: "Closed" }),
      isLoading: false,
      isError: false,
    });

    const { getByTestId, container } = renderBubble({
      kind: "child_unblocked",
      child_id: "i-childex",
      new_status: "closed",
    });

    const chip = getByTestId("system-event-child-unblocked-chip");
    expect(chip.getAttribute("data-child-id")).toBe("i-childex");
    expect(chip.getAttribute("href")).toBe("/issues/i-childex");
    expect(chip.textContent).toContain("Re-index search corpus");
    expect(chip.textContent).toContain("Closed");
    expect(container.querySelector('[data-testid="system-event-bubble"]')).not.toBeNull();
  });

  it("falls back to the raw child_id and new_status key while the issue query is still loading", () => {
    useIssueMock.mockReturnValue({
      data: undefined,
      isLoading: true,
      isError: false,
    });

    const { getByTestId } = renderBubble({
      kind: "child_unblocked",
      child_id: "i-childex",
      new_status: "complete",
    });

    const chip = getByTestId("system-event-child-unblocked-chip");
    expect(chip.textContent).toContain("i-childex");
    expect(chip.textContent).toContain("complete");
  });

  it("uses the parent's resolved status_key fallback when the issue query errors", () => {
    useIssueMock.mockReturnValue({
      data: undefined,
      isLoading: false,
      isError: true,
    });

    const { getByTestId } = renderBubble({
      kind: "child_unblocked",
      child_id: "i-broken",
      new_status: "failed",
    });

    expect(getByTestId("system-event-child-unblocked-chip").textContent).toContain(
      "failed",
    );
  });
});

describe("SystemEventBubble — forward-compat fallback", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders a generic 'System event' line for an unrecognized kind", () => {
    // `SystemEventKind` is `#[non_exhaustive]` on the Rust side; ts-rs only
    // emits the currently-known variants. Cast through `unknown` to model a
    // wire payload from a newer backend than the frontend knows about.
    const futureKind = {
      kind: "patch_reviewed",
      patch_id: "p-future01",
      decision: "approved",
    } as unknown as SystemEventKind;

    const { getByTestId, queryByTestId } = renderBubble(futureKind, "2026-06-12T00:00:00Z");

    const row = getByTestId("system-event-bubble");
    expect(row.getAttribute("data-kind")).toBe("patch_reviewed");
    expect(row.textContent).toContain("System event");
    expect(queryByTestId("system-event-child-unblocked-chip")).toBeNull();
  });
});
