// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, within } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import type {
  ProjectRecord,
  ProjectStatusesResponse,
  StatusDefinition,
} from "@hydra/api";

// --- Hook mocks ---

let projectsData: ProjectRecord[] | undefined = [];
let defaultStatusesData: ProjectStatusesResponse | undefined = {
  statuses: [],
  default_status_key: "open",
};

vi.mock("../../../projects/useProjects", () => ({
  useProjects: () => ({ data: projectsData }),
  useProjectStatuses: () => ({ data: defaultStatusesData }),
}));

vi.mock("../../usePaginatedIssues", () => ({
  useBoardIssuesByProject: () => new Map(),
}));

vi.mock("../../../dashboard/usePageIssueTrees", () => ({
  usePageIssueTrees: () => ({
    isActiveMap: new Map(),
    childStatusMap: new Map(),
    sessionsByIssue: new Map(),
    isLoading: false,
  }),
}));

// --- @hydra/ui stubs ---
vi.mock("@hydra/ui", () => ({
  Avatar: ({ name, kind }: { name: string; kind?: string }) => (
    <span data-testid={`avatar-${kind ?? "human"}`}>{name}</span>
  ),
  TypeChip: ({ type }: { type: string }) => <span>{type}</span>,
}));

// Bypass CSS module proxies.
vi.mock("../IssuesBoard.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("../../../projects/StatusChip.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("../../../projects/ProjectChip.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const { IssuesBoard } = await import("../IssuesBoard");

// --- Fixtures ---

function makeStatus(
  overrides: Partial<StatusDefinition> & Pick<StatusDefinition, "key" | "label">,
): StatusDefinition {
  return {
    color: "#3498db",
    unblocks_parents: false,
    unblocks_dependents: false,
    cascades_to_children: false,
    ...overrides,
  };
}

function makeProject(
  id: string,
  key: string,
  statuses: StatusDefinition[],
): ProjectRecord {
  return {
    project_id: id,
    version: 1,
    project: {
      key,
      name: key,
      statuses,
      default_status_key: statuses[0]?.key ?? "open",
      creator: "alice",
      deleted: false,
    },
  };
}

function renderBoard() {
  return render(
    <MemoryRouter>
      <IssuesBoard
        baseFilters={{}}
        username="alice"
        filterRootId={null}
      />
    </MemoryRouter>,
  );
}

beforeEach(() => {
  projectsData = [];
  defaultStatusesData = { statuses: [], default_status_key: "open" };
});

describe("IssuesBoard column sub-row", () => {
  it("renders the assignee avatar+name for a status with on_enter.assign_to = User", () => {
    const status = makeStatus({
      key: "review",
      label: "Review",
      on_enter: { assign_to: { User: { name: "alice" } } },
    });
    projectsData = [makeProject("j-proj", "proj", [status])];

    renderBoard();

    const subhead = screen.getByTestId("board-col-subhead-proj-review");
    expect(within(subhead).getByTestId("avatar-human").textContent).toBe("alice");
    expect(within(subhead).getAllByText("alice").length).toBeGreaterThan(0);
  });

  it("renders the assignee avatar+name for a status with on_enter.assign_to = Agent", () => {
    const status = makeStatus({
      key: "doing",
      label: "Doing",
      on_enter: { assign_to: { Agent: { name: "swe" } } },
    });
    projectsData = [makeProject("j-proj", "proj", [status])];

    renderBoard();

    const subhead = screen.getByTestId("board-col-subhead-proj-doing");
    expect(within(subhead).getByTestId("avatar-agent").textContent).toBe("swe");
    expect(within(subhead).getAllByText("swe").length).toBeGreaterThan(0);
  });

  it("renders 'auto' badge and no avatar for a status with no on_enter", () => {
    const status = makeStatus({ key: "open", label: "Open" });
    projectsData = [makeProject("j-proj", "proj", [status])];

    renderBoard();

    const subhead = screen.getByTestId("board-col-subhead-proj-open");
    expect(within(subhead).queryByTestId("avatar-human")).toBeNull();
    expect(within(subhead).queryByTestId("avatar-agent")).toBeNull();
    const mode = within(subhead).getByTestId("board-col-mode-proj-open");
    expect(mode.textContent).toBe("auto");
  });

  it("renders 'interactive' badge when status.interactive === true", () => {
    const status = makeStatus({
      key: "triage",
      label: "Triage",
      interactive: true,
    });
    projectsData = [makeProject("j-proj", "proj", [status])];

    renderBoard();

    const mode = screen.getByTestId("board-col-mode-proj-triage");
    expect(mode.textContent).toBe("interactive");
  });

  it("keeps the DEFAULT chip on the top header row, not the sub-row", () => {
    const status = makeStatus({ key: "open", label: "Open" });
    projectsData = [makeProject("j-proj", "proj", [status])];

    renderBoard();

    const subhead = screen.getByTestId("board-col-subhead-proj-open");
    expect(within(subhead).queryByText("DEFAULT")).toBeNull();
    // DEFAULT chip lives elsewhere in the column (the top header row).
    expect(screen.getByText("DEFAULT")).toBeDefined();
  });
});
