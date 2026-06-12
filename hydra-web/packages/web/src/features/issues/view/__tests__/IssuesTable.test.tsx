// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import type {
  IssueSummaryRecord,
  ProjectRecord,
  SessionSummaryRecord,
  StatusDefinition,
} from "@hydra/api";
import type { IssueNeighborhood } from "../../flowPill";
import { makeStatusDef } from "../../../../test-utils/statusDef";

// --- Hook mocks ---

// Force desktop branch by default. Tests that need mobile override this.
let mobileMatches = false;
vi.mock("../../../../hooks/useMediaQuery", () => ({
  useMediaQuery: () => mobileMatches,
}));

let projectsData: ProjectRecord[] | undefined = [];
vi.mock("../../../projects/useProjects", () => ({
  useProjects: () => ({ data: projectsData }),
}));

vi.mock("../../../dashboard/useSessionDuration", () => ({
  useSessionDuration: () => ({ durationText: "—", status: "idle" }),
}));

// --- @hydra/ui stubs ---
vi.mock("@hydra/ui", () => ({
  Avatar: ({ name }: { name: string }) => <span data-testid="avatar">{name}</span>,
  TypeChip: ({ type }: { type: string }) => <span data-testid={`type-${type}`}>{type}</span>,
  StatusDot: () => <span data-testid="status-dot" />,
  FlowPill: ({
    phase,
    num,
    den,
    "data-testid": testId,
  }: {
    phase: string;
    num: number;
    den: number;
    "data-testid"?: string;
  }) => (
    <span data-testid={testId ?? "flowpill"} data-phase={phase}>
      {num}/{den}
    </span>
  ),
  Icons: {
    IconDoc: () => <span />,
    IconRepo: () => <span />,
  },
}));

// Stub child rail-row so mobile-branch assertions are simple.
vi.mock("../../../related/RailRow", () => ({
  IssueRailRow: ({ record }: { record: IssueSummaryRecord }) => (
    <div data-testid={`rail-row-${record.issue_id}`}>{record.issue.title}</div>
  ),
}));

// Stub the row-action buttons; both require a QueryClientProvider / Toast
// context that the desktop-table tests don't otherwise need.
vi.mock("../../ArchiveIssueButton", () => ({
  ArchiveIssueButton: ({ "data-testid": testId }: { "data-testid"?: string }) => (
    <button data-testid={testId}>Archive</button>
  ),
}));

vi.mock("../../RestoreIssueButton", () => ({
  RestoreIssueButton: ({ "data-testid": testId }: { "data-testid"?: string }) => (
    <button data-testid={testId}>Restore</button>
  ),
}));

// Bypass CSS module proxy.
vi.mock("../IssuesTable.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("../../../projects/StatusChip.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("../../../projects/ProjectChip.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const { IssuesTable } = await import("../IssuesTable");

// --- Fixtures ---

const STATUSES: StatusDefinition[] = [
  {
    key: "open",
    label: "Open",
    color: "#3498db",
    unblocks_parents: false,
    unblocks_dependents: false,
    cascades_to_children: false,
    position: 0,
  },
  {
    key: "in-progress",
    label: "In progress",
    color: "#f1c40f",
    unblocks_parents: false,
    unblocks_dependents: false,
    cascades_to_children: false,
    position: 0,
  },
  {
    key: "closed",
    label: "Closed",
    color: "#2ecc71",
    unblocks_parents: true,
    unblocks_dependents: true,
    cascades_to_children: false,
    position: 0,
  },
];

const ALT_STATUSES: StatusDefinition[] = [
  {
    key: "open",
    label: "Open",
    color: "#9b59b6",
    unblocks_parents: false,
    unblocks_dependents: false,
    cascades_to_children: false,
    position: 0,
  },
];

function makeProject(
  id: string,
  key: string,
  name: string,
  statuses: StatusDefinition[] = STATUSES,
): ProjectRecord {
  return {
    project_id: id,
    version: 1,
    project: {
      key,
      name,
      statuses,
      creator: "alice",
      deleted: false,
      priority: 0,
    },
  };
}

function makeIssue(
  id: string,
  overrides: Omit<Partial<IssueSummaryRecord["issue"]>, "status"> & { status?: string } = {},
): IssueSummaryRecord {
  const { status, ...rest } = overrides;
  return {
    issue_id: id,
    version: 1n,
    timestamp: "2026-06-01T00:00:00Z",
    creation_time: "2026-06-01T00:00:00Z",
    issue: {
      type: "task",
      title: `Issue ${id}`,
      description: "",
      creator: "alice",
      status: makeStatusDef(status ?? "open"),
      project_id: "j-defaul",
      assignee: null,
      progress: "",
      dependencies: [],
      patches: [],
      labels: [],
      ...rest,
    },
  };
}

function renderTable(props: {
  issues: IssueSummaryRecord[];
  neighborhoodMap?: Map<string, IssueNeighborhood>;
  sessionsByIssue?: Map<string, SessionSummaryRecord[]>;
  filterRootId?: string | null;
}) {
  return render(
    <MemoryRouter>
      <IssuesTable
        issues={props.issues}
        neighborhoodMap={props.neighborhoodMap ?? new Map()}
        sessionsByIssue={props.sessionsByIssue ?? new Map()}
        filterRootId={props.filterRootId ?? null}
      />
    </MemoryRouter>,
  );
}

beforeEach(() => {
  mobileMatches = false;
  projectsData = [
    makeProject("j-defaul", "default", "Default"),
    makeProject("j-altpro", "alpha", "Alpha", ALT_STATUSES),
  ];
});

describe("IssuesTable grouping", () => {
  it("groups issues under per-project section headers", () => {
    renderTable({
      issues: [
        makeIssue("i-a", { project_id: "j-defaul", status: "open" }),
        makeIssue("i-b", { project_id: "j-defaul", status: "in-progress" }),
        makeIssue("i-c", { project_id: "j-altpro", status: "open" }),
      ],
    });

    const defaultGroup = screen.getByTestId("issues-table-group-default");
    const alphaGroup = screen.getByTestId("issues-table-group-alpha");
    expect(defaultGroup).toBeDefined();
    expect(alphaGroup).toBeDefined();

    expect(screen.getByTestId("issues-list-row-i-a")).toBeDefined();
    expect(screen.getByTestId("issues-list-row-i-b")).toBeDefined();
    expect(screen.getByTestId("issues-list-row-i-c")).toBeDefined();
  });

  it("places issues with the seeded default project_id under the default section", () => {
    renderTable({
      issues: [makeIssue("i-null", { project_id: "j-defaul", status: "open" })],
    });

    const defaultGroup = screen.getByTestId("issues-table-group-default");
    expect(defaultGroup).toBeDefined();
    expect(screen.getByTestId("issues-list-row-i-null")).toBeDefined();
  });

  it("falls back to a flat ungrouped table when projects haven't loaded", () => {
    projectsData = undefined;
    renderTable({
      issues: [
        makeIssue("i-a", { project_id: "j-defaul" }),
        makeIssue("i-b", { project_id: "j-altpro" }),
      ],
    });

    expect(screen.queryByTestId("issues-table-group-default")).toBeNull();
    expect(screen.getByTestId("issues-list-row-i-a")).toBeDefined();
    expect(screen.getByTestId("issues-list-row-i-b")).toBeDefined();
  });
});

describe("IssuesTable collapse toggle", () => {
  it("hides issue rows under a group when its header is clicked", () => {
    renderTable({
      issues: [
        makeIssue("i-a", { project_id: "j-defaul" }),
        makeIssue("i-c", { project_id: "j-altpro" }),
      ],
    });

    expect(screen.getByTestId("issues-list-row-i-a")).toBeDefined();

    fireEvent.click(screen.getByTestId("issues-table-group-toggle-default"));

    expect(screen.queryByTestId("issues-list-row-i-a")).toBeNull();
    // Other groups stay visible.
    expect(screen.getByTestId("issues-list-row-i-c")).toBeDefined();

    fireEvent.click(screen.getByTestId("issues-table-group-toggle-default"));
    expect(screen.getByTestId("issues-list-row-i-a")).toBeDefined();
  });
});

describe("IssuesTable status pips", () => {
  it("renders a pip for each non-zero status in the loaded section issues, in project status order", () => {
    renderTable({
      issues: [
        makeIssue("i-a", { project_id: "j-defaul", status: "open" }),
        makeIssue("i-b", { project_id: "j-defaul", status: "open" }),
        makeIssue("i-c", { project_id: "j-defaul", status: "in-progress" }),
      ],
    });

    const openPip = screen.getByTestId("issues-table-pip-default-open");
    const progPip = screen.getByTestId("issues-table-pip-default-in-progress");
    expect(openPip).toBeDefined();
    expect(progPip).toBeDefined();

    expect(within(openPip).getByText("2")).toBeDefined();
    expect(within(progPip).getByText("1")).toBeDefined();

    // Zero-count status is skipped.
    expect(screen.queryByTestId("issues-table-pip-default-closed")).toBeNull();
  });
});

describe("IssuesTable BLOCKED tag", () => {
  it("never renders a BLOCKED tag on issue rows, even when a blocked-on dep is open", () => {
    const blocker = makeIssue("i-blocker", {
      project_id: "j-defaul",
      status: "open",
    });
    const blocked = makeIssue("i-blocked", {
      project_id: "j-defaul",
      status: "open",
      dependencies: [{ type: "blocked-on", issue_id: "i-blocker" }],
    });

    renderTable({ issues: [blocker, blocked] });

    expect(screen.queryByTestId("issues-row-blocked-i-blocked")).toBeNull();
    expect(screen.queryByTestId("issues-row-blocked-i-blocker")).toBeNull();
  });
});

describe("IssuesTable mobile layout", () => {
  it("keeps per-project section headers above mobile rail rows", () => {
    mobileMatches = true;
    renderTable({
      issues: [
        makeIssue("i-a", { project_id: "j-defaul" }),
        makeIssue("i-c", { project_id: "j-altpro" }),
      ],
    });

    expect(screen.getByTestId("issues-table-group-default")).toBeDefined();
    expect(screen.getByTestId("issues-table-group-alpha")).toBeDefined();
    expect(screen.getByTestId("rail-row-i-a")).toBeDefined();
    expect(screen.getByTestId("rail-row-i-c")).toBeDefined();
  });
});
