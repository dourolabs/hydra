// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import type {
  ProjectRecord,
  ProjectStatusesResponse,
  StatusDefinition,
} from "@hydra/api";
import type { BoardCellQuery } from "../../usePaginatedIssues";

// --- Hook mocks ---

let projectsData: ProjectRecord[] | undefined = [];
let defaultStatusesData: ProjectStatusesResponse | undefined = {
  statuses: [],
  default_status_key: "open",
};
let cellsByProject: Map<string | null, Map<string, BoardCellQuery>> = new Map();

vi.mock("../../../projects/useProjects", () => ({
  useProjects: () => ({ data: projectsData }),
  useProjectStatuses: () => ({ data: defaultStatusesData }),
}));

vi.mock("../../usePaginatedIssues", () => ({
  useBoardIssuesByProject: () => cellsByProject,
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
  Modal: ({
    open,
    onClose,
    title,
    children,
  }: {
    open: boolean;
    onClose: () => void;
    title?: string;
    children: React.ReactNode;
  }) =>
    open ? (
      <div data-testid="modal" role="dialog" aria-label={title}>
        <div data-testid="modal-title">{title}</div>
        <button data-testid="modal-close" onClick={onClose}>
          Close
        </button>
        <div>{children}</div>
      </div>
    ) : null,
  Icons: {
    IconSettings: () => <span data-testid="icon-settings" />,
  },
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

vi.mock("../../../../components/LargeModal.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

// Replace ProjectEditor with a sentinel that captures the props received,
// so the modal-side assertions stay focused on wiring (which project's
// data is in the modal, not the editor internals).
vi.mock("../../../projects/ProjectEditor", () => ({
  ProjectEditor: ({
    projectId,
    initial,
    creator,
  }: {
    projectId?: string | null;
    initial?: { key: string; name: string };
    creator: string;
  }) => (
    <div
      data-testid="project-editor"
      data-project-id={String(projectId ?? "")}
      data-project-key={initial?.key ?? ""}
      data-project-name={initial?.name ?? ""}
      data-creator={creator}
    />
  ),
}));

const lastModalProps: {
  projectRecord?: ProjectRecord;
  statusKey?: string;
  issueCount?: number;
  open?: boolean;
} = {};
vi.mock("../../../projects/StatusSettingsModal", () => ({
  StatusSettingsModal: ({
    open,
    projectRecord,
    statusKey,
    issueCount,
    onClose,
  }: {
    open: boolean;
    projectRecord: ProjectRecord;
    statusKey: string;
    issueCount: number;
    onClose: () => void;
  }) => {
    lastModalProps.open = open;
    lastModalProps.projectRecord = projectRecord;
    lastModalProps.statusKey = statusKey;
    lastModalProps.issueCount = issueCount;
    return open ? (
      <div data-testid="status-settings-modal">
        modal:{projectRecord.project_id}:{statusKey}:{issueCount}
        <button data-testid="status-modal-close" onClick={onClose}>
          x
        </button>
      </div>
    ) : null;
  },
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

const DEFAULT_STATUSES: StatusDefinition[] = [
  makeStatus({ key: "open", label: "Open" }),
];

const ENG_STATUSES: StatusDefinition[] = [
  makeStatus({ key: "open", label: "Open", color: "#9b59b6" }),
  makeStatus({ key: "in-progress", label: "In progress", color: "#f1c40f" }),
];

function makeProject(
  id: string,
  key: string,
  statuses: StatusDefinition[],
  name?: string,
): ProjectRecord {
  return {
    project_id: id,
    version: 1,
    project: {
      key,
      name: name ?? key,
      statuses,
      default_status_key: statuses[0]?.key ?? "open",
      creator: "alice",
      deleted: false,
    },
  };
}

function emptyCell(overrides: Partial<BoardCellQuery> = {}): BoardCellQuery {
  return {
    issues: [],
    isLoading: false,
    hasNextPage: false,
    isFetchingNextPage: false,
    fetchNextPage: () => {},
    ...overrides,
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
  cellsByProject = new Map();
  lastModalProps.open = undefined;
  lastModalProps.projectRecord = undefined;
  lastModalProps.statusKey = undefined;
  lastModalProps.issueCount = undefined;
});

afterEach(() => {
  cleanup();
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

describe("IssuesBoard project settings gear", () => {
  beforeEach(() => {
    projectsData = [
      makeProject("j-altpro", "alpha", DEFAULT_STATUSES, "Alpha"),
      makeProject("j-betpro", "beta", DEFAULT_STATUSES, "Beta"),
    ];
    defaultStatusesData = {
      statuses: DEFAULT_STATUSES,
      default_status_key: "open",
    };
  });

  it("renders the gear button on each real project section", () => {
    renderBoard();

    expect(screen.getByTestId("board-project-settings-alpha")).toBeDefined();
    expect(screen.getByTestId("board-project-settings-beta")).toBeDefined();
  });

  it("does NOT render the gear on the synthesized default-project section", () => {
    renderBoard();

    // The default section is synthesized from `defaultStatusesData` with
    // project_id: null — it must not expose a gear because there is no
    // real ProjectRecord to edit.
    expect(screen.getByTestId("board-project-default")).toBeDefined();
    expect(screen.queryByTestId("board-project-settings-default")).toBeNull();
  });

  it("opens the settings modal with the clicked project's data", () => {
    renderBoard();

    expect(screen.queryByTestId("modal")).toBeNull();

    fireEvent.click(screen.getByTestId("board-project-settings-alpha"));

    const modal = screen.getByTestId("modal");
    expect(modal).toBeDefined();
    const editor = screen.getByTestId("project-editor");
    expect(editor.getAttribute("data-project-id")).toBe("j-altpro");
    expect(editor.getAttribute("data-project-key")).toBe("alpha");
    expect(editor.getAttribute("data-project-name")).toBe("Alpha");
    expect(editor.getAttribute("data-creator")).toBe("alice");
  });

  it("dismisses the modal when close is clicked", () => {
    renderBoard();

    fireEvent.click(screen.getByTestId("board-project-settings-beta"));
    expect(screen.getByTestId("modal")).toBeDefined();

    fireEvent.click(screen.getByTestId("modal-close"));
    expect(screen.queryByTestId("modal")).toBeNull();
  });
});

describe("IssuesBoard column gear", () => {
  beforeEach(() => {
    projectsData = [makeProject("j-eng", "engineering", ENG_STATUSES, "Engineering")];
    defaultStatusesData = {
      statuses: DEFAULT_STATUSES,
      default_status_key: "open",
    };
    cellsByProject = new Map([
      [
        null,
        new Map<string, BoardCellQuery>([["open", emptyCell()]]),
      ],
      [
        "j-eng",
        new Map<string, BoardCellQuery>([
          ["open", emptyCell()],
          ["in-progress", emptyCell()],
        ]),
      ],
    ]);
  });

  it("renders a gear button per column for real-project sections", () => {
    renderBoard();

    expect(screen.getByTestId("board-col-gear-engineering-open")).toBeDefined();
    expect(
      screen.getByTestId("board-col-gear-engineering-in-progress"),
    ).toBeDefined();
  });

  it("suppresses the gear for the synthesized default-project section", () => {
    renderBoard();
    // The synthesized default section has no ProjectRecord; the gear is omitted.
    expect(screen.queryByTestId("board-col-gear-default-open")).toBeNull();
  });

  it("opens StatusSettingsModal with the right status key on gear click", () => {
    renderBoard();
    fireEvent.click(
      screen.getByTestId("board-col-gear-engineering-in-progress"),
    );

    expect(screen.getByTestId("status-settings-modal")).toBeDefined();
    expect(lastModalProps.statusKey).toBe("in-progress");
    expect(lastModalProps.projectRecord?.project_id).toBe("j-eng");
    expect(lastModalProps.issueCount).toBe(0);
  });

  it("treats hasNextPage as a non-empty column for delete safety", () => {
    cellsByProject = new Map([
      [
        "j-eng",
        new Map<string, BoardCellQuery>([
          ["open", emptyCell({ hasNextPage: true })],
          ["in-progress", emptyCell()],
        ]),
      ],
    ]);
    renderBoard();
    fireEvent.click(screen.getByTestId("board-col-gear-engineering-open"));
    // hasNextPage forces a sentinel positive count so the modal disables delete.
    expect(lastModalProps.issueCount).toBeGreaterThan(0);
  });
});
