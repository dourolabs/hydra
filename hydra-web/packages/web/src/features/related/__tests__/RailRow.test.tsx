import { describe, it, expect, vi, beforeEach } from "vitest";
import { render } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import type { IssueSummaryRecord, Principal, ProjectRecord, StatusDefinition } from "@hydra/api";

const useProjectsMock = vi.fn<() => { data: ProjectRecord[] | undefined }>();
vi.mock("../../projects/useProjects", () => ({
  useProjects: () => useProjectsMock(),
}));

const { IssueRailRow } = await import("../RailRow");

const SEEDED_PROJECTS: ProjectRecord[] = [
  {
    project_id: "j-defaul",
    version: 1,
    project: {
      key: "default",
      name: "Default",
      statuses: [],
      creator: "system",
      priority: 0,
    },
  },
  {
    project_id: "j-engv2",
    version: 1,
    project: {
      key: "engineering-v2",
      name: "Engineering v2",
      statuses: [],
      creator: "alice",
      priority: 0,
    },
  },
];

function makeStatus(over?: Partial<StatusDefinition>): StatusDefinition {
  return {
    key: "open",
    label: "Open",
    color: "#3498db",
    unblocks_parents: false,
    unblocks_dependents: false,
    cascades_to_children: false,
    ...over,
  };
}

function makeRecord(opts?: {
  assignee?: Principal | null;
  projectId?: string | null;
  resolvedStatus?: StatusDefinition | null;
}): IssueSummaryRecord {
  return {
    issue_id: "i-1",
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    issue: {
      type: "task",
      title: "Wire avatar",
      description: "desc",
      creator: "alice",
      status: "open",
      project_id: opts?.projectId ?? null,
      resolved_status:
        opts?.resolvedStatus === undefined ? makeStatus() : opts.resolvedStatus,
      assignee: opts?.assignee ?? null,
      progress: "",
      dependencies: [],
      patches: [],
      labels: [],
    },
  };
}

function renderRow(record: IssueSummaryRecord) {
  return render(
    <MemoryRouter>
      <IssueRailRow record={record} />
    </MemoryRouter>,
  );
}

describe("IssueRailRow assignee avatar", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useProjectsMock.mockReturnValue({ data: SEEDED_PROJECTS });
  });

  it("does not render an avatar when assignee is absent", () => {
    const { container } = renderRow(makeRecord({ assignee: null }));
    const avatars = container.querySelectorAll('[title^="Assignee"]');
    expect(avatars).toHaveLength(0);
  });

  it("renders an avatar with human kind and the Assignee tooltip for a User principal", () => {
    const principal: Principal = { User: { name: "alice" } };
    const { container } = renderRow(makeRecord({ assignee: principal }));
    const avatar = container.querySelector('[title="Assignee · alice"]');
    expect(avatar).not.toBeNull();
    expect(avatar?.getAttribute("data-kind")).toBe("human");
    expect(avatar?.getAttribute("aria-label")).toBe("Assignee · alice");
  });

  it("renders an avatar with agent kind for an Agent principal", () => {
    const principal: Principal = { Agent: { name: "swe" } };
    const { container } = renderRow(makeRecord({ assignee: principal }));
    const avatar = container.querySelector('[title="Assignee · swe"]');
    expect(avatar).not.toBeNull();
    expect(avatar?.getAttribute("data-kind")).toBe("agent");
  });

  it("places the avatar before the AgoTime element in the meta line", () => {
    const principal: Principal = { User: { name: "alice" } };
    const { container } = renderRow(makeRecord({ assignee: principal }));
    const meta = container.querySelector('[class*="meta"]');
    expect(meta).not.toBeNull();
    const children = Array.from(meta!.children);
    const avatarIdx = children.findIndex((el) => el.getAttribute("title") === "Assignee · alice");
    expect(avatarIdx).toBeGreaterThan(0);
    const agoIdx = children.findIndex((el) =>
      (el.getAttribute("title") ?? "").startsWith("Last updated"),
    );
    expect(agoIdx).toBeGreaterThan(avatarIdx);
  });
});

describe("IssueRailRow ProjectChip + status label", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useProjectsMock.mockReturnValue({ data: SEEDED_PROJECTS });
  });

  it("renders a ProjectChip with the default project key when project_id is null", () => {
    const { getByTestId } = renderRow(makeRecord({ projectId: null }));
    const chip = getByTestId("rail-row-project-chip-i-1");
    expect(chip.textContent).toBe("default");
  });

  it("renders a ProjectChip with the matched project key when project_id is set", () => {
    const { getByTestId } = renderRow(makeRecord({ projectId: "j-engv2" }));
    const chip = getByTestId("rail-row-project-chip-i-1");
    expect(chip.textContent).toBe("engineering-v2");
  });

  it("renders the resolved_status label as a supplementary mono span", () => {
    const { container } = renderRow(
      makeRecord({
        resolvedStatus: makeStatus({
          key: "in-progress",
          label: "In progress",
          color: "#f1c40f",
        }),
      }),
    );
    const meta = container.querySelector('[class*="meta"]');
    expect(meta?.textContent).toContain("In progress");
  });

  it("places the ProjectChip before the TypeChip in the meta row", () => {
    const { container, getByTestId } = renderRow(makeRecord({ projectId: "j-engv2" }));
    const meta = container.querySelector('[class*="meta"]');
    expect(meta).not.toBeNull();
    const children = Array.from(meta!.children);
    const chipIdx = children.indexOf(getByTestId("rail-row-project-chip-i-1"));
    const typeChipIdx = children.findIndex((el) =>
      (el.getAttribute("class") ?? "").toLowerCase().includes("type"),
    );
    expect(chipIdx).toBeGreaterThanOrEqual(0);
    expect(typeChipIdx).toBeGreaterThan(chipIdx);
  });

  it("does not render a ProjectChip when projects list is empty", () => {
    useProjectsMock.mockReturnValue({ data: [] });
    const { queryByTestId } = renderRow(makeRecord({ projectId: null }));
    expect(queryByTestId("rail-row-project-chip-i-1")).toBeNull();
  });
});
