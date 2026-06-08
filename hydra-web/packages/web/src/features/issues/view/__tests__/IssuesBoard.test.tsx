// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type {
  ListProjectsResponse,
  ProjectRecord,
  StatusDefinition,
} from "@hydra/api";
import type { BoardCellQuery } from "../../usePaginatedIssues";

// --- Hook mocks ---

let projectsData: ProjectRecord[] | undefined = [];
let cellsByProject: Map<string, Map<string, BoardCellQuery>> = new Map();

vi.mock("../../../projects/useProjects", () => ({
  useProjects: () => ({ data: projectsData }),
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

// Capture the DnD onDragEnd handlers so tests can synthesize drops without
// driving a real pointer/keyboard event flow. Mocking out the dnd-kit
// runtime here also keeps `useSortable` from requiring a real DOM measurer.
// `lastDragEndHandler` and `lastSortableItems` keep the historical
// "last-rendered" semantics (= innermost = status-level when sections
// present). For project-level (outer) drag tests, use the parallel arrays.
let lastDragEndHandler: ((event: unknown) => void) | null = null;
let lastSortableItems: unknown[] = [];
let dragEndHandlers: Array<(event: unknown) => void> = [];
let sortableItemsList: unknown[][] = [];
vi.mock("@dnd-kit/core", () => ({
  DndContext: ({
    children,
    onDragEnd,
  }: {
    children: React.ReactNode;
    onDragEnd?: (event: unknown) => void;
  }) => {
    if (onDragEnd) {
      dragEndHandlers.push(onDragEnd);
      lastDragEndHandler = onDragEnd;
    }
    return <>{children}</>;
  },
  PointerSensor: function PointerSensor() {},
  KeyboardSensor: function KeyboardSensor() {},
  useSensor: () => ({}),
  useSensors: () => [],
  closestCenter: () => [],
}));

vi.mock("@dnd-kit/sortable", () => ({
  SortableContext: ({
    children,
    items,
  }: {
    children: React.ReactNode;
    items: unknown[];
  }) => {
    sortableItemsList.push(items);
    lastSortableItems = items;
    return <>{children}</>;
  },
  useSortable: () => ({
    attributes: { "data-sortable-handle": "true", tabIndex: 0, role: "button" },
    listeners: {},
    setNodeRef: () => {},
    transform: null,
    transition: null,
    isDragging: false,
    isOver: false,
  }),
  arrayMove: <T,>(arr: T[], from: number, to: number) => {
    const next = arr.slice();
    next.splice(to, 0, next.splice(from, 1)[0]);
    return next;
  },
  horizontalListSortingStrategy: function strategy() {},
  verticalListSortingStrategy: function vstrategy() {},
  sortableKeyboardCoordinates: function coords() {},
}));

const mockUpdateProject = vi.fn();
vi.mock("../../../../api/client", () => ({
  apiClient: {
    updateProject: (
      projectId: string,
      request: { project: { statuses: StatusDefinition[] } },
    ) => mockUpdateProject(projectId, request),
  },
}));

const mockAddToast = vi.fn();
vi.mock("../../../toast/useToast", () => ({
  useToast: () => ({ addToast: mockAddToast }),
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

// ProjectCreateModal calls useUsername(); render it as "alice" so the
// modal renders.
vi.mock("../../../auth/useUsername", () => ({
  useUsername: () => "alice",
}));

// Replace ProjectEditor with a sentinel — the settings (edit) modal still
// uses it. The new-project modal doesn't, so a separate sentinel covers
// that route.
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

vi.mock("../../../projects/ProjectCreateModal", () => ({
  ProjectCreateModal: ({ open }: { open: boolean }) =>
    open ? <div data-testid="new-project-modal" /> : null,
}));

const lastModalProps: {
  projectRecord?: ProjectRecord;
  statusKey?: string;
  issueCount?: number;
  open?: boolean;
  mode?: "edit" | "new";
} = {};
vi.mock("../../../projects/StatusSettingsModal", () => ({
  StatusSettingsModal: ({
    open,
    projectRecord,
    statusKey,
    issueCount,
    mode,
    onClose,
  }: {
    open: boolean;
    projectRecord: ProjectRecord;
    statusKey?: string;
    issueCount?: number;
    mode?: "edit" | "new";
    onClose: () => void;
  }) => {
    lastModalProps.open = open;
    lastModalProps.projectRecord = projectRecord;
    lastModalProps.statusKey = statusKey;
    lastModalProps.issueCount = issueCount;
    lastModalProps.mode = mode ?? "edit";
    return open ? (
      <div data-testid="status-settings-modal" data-mode={mode ?? "edit"}>
        modal:{projectRecord.project_id}:{statusKey ?? ""}:{issueCount ?? 0}
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
      creator: "alice",
      deleted: false,
      priority: 0,
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

function renderBoard(
  queryClient?: QueryClient,
  opts: { hideIssues?: boolean } = {},
) {
  const client =
    queryClient ??
    new QueryClient({
      defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
    });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <IssuesBoard
          baseFilters={{}}
          username="alice"
          filterRootId={null}
          hideIssues={opts.hideIssues}
        />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  projectsData = [];
  cellsByProject = new Map();
  lastModalProps.open = undefined;
  lastModalProps.projectRecord = undefined;
  lastModalProps.statusKey = undefined;
  lastModalProps.issueCount = undefined;
  lastDragEndHandler = null;
  lastSortableItems = [];
  dragEndHandlers = [];
  sortableItemsList = [];
  mockUpdateProject.mockReset();
  mockUpdateProject.mockResolvedValue({ project_id: "j-eng", version: 2 });
  mockAddToast.mockReset();
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

});

describe("IssuesBoard project settings gear", () => {
  beforeEach(() => {
    projectsData = [
      makeProject("j-altpro", "alpha", DEFAULT_STATUSES, "Alpha"),
      makeProject("j-betpro", "beta", DEFAULT_STATUSES, "Beta"),
    ];
  });

  it("renders the gear button on each project section", () => {
    renderBoard();

    expect(screen.getByTestId("board-project-settings-alpha")).toBeDefined();
    expect(screen.getByTestId("board-project-settings-beta")).toBeDefined();
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
    cellsByProject = new Map([
      [
        "j-eng",
        new Map<string, BoardCellQuery>([
          ["open", emptyCell()],
          ["in-progress", emptyCell()],
        ]),
      ],
    ]);
  });

  it("renders a gear button per column", () => {
    renderBoard();

    expect(screen.getByTestId("board-col-gear-engineering-open")).toBeDefined();
    expect(
      screen.getByTestId("board-col-gear-engineering-in-progress"),
    ).toBeDefined();
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

describe("IssuesBoard '+ Add status' ghost column", () => {
  beforeEach(() => {
    projectsData = [
      makeProject("j-eng", "engineering", ENG_STATUSES, "Engineering"),
    ];
  });

  it("renders the '+ Add status' ghost column for each project", () => {
    renderBoard();
    expect(screen.getByTestId("board-col-add-engineering")).toBeDefined();
  });

  it("opens StatusSettingsModal in 'new' mode for the clicked project", () => {
    renderBoard();
    expect(screen.queryByTestId("status-settings-modal")).toBeNull();

    fireEvent.click(screen.getByTestId("board-col-add-engineering"));

    const modal = screen.getByTestId("status-settings-modal");
    expect(modal.getAttribute("data-mode")).toBe("new");
    expect(lastModalProps.mode).toBe("new");
    expect(lastModalProps.projectRecord?.project_id).toBe("j-eng");
    expect(lastModalProps.statusKey).toBeUndefined();
  });

  it("dismisses the new-status modal when close is invoked", () => {
    renderBoard();
    fireEvent.click(screen.getByTestId("board-col-add-engineering"));
    expect(screen.getByTestId("status-settings-modal")).toBeDefined();

    fireEvent.click(screen.getByTestId("status-modal-close"));
    expect(screen.queryByTestId("status-settings-modal")).toBeNull();
  });
});

describe("IssuesBoard '+ New project' ghost row", () => {
  beforeEach(() => {
    projectsData = [
      makeProject("j-eng", "engineering", ENG_STATUSES, "Engineering"),
    ];
  });

  it("renders the '+ New project' row at the end of the board when not scoped", () => {
    renderBoard();
    expect(screen.getByTestId("board-new-project")).toBeDefined();
  });

  it("is suppressed when the board is scoped to a single project", () => {
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
    });
    render(
      <QueryClientProvider client={client}>
        <MemoryRouter>
          <IssuesBoard
            baseFilters={{ project_id: "j-eng" }}
            username="alice"
            filterRootId={null}
          />
        </MemoryRouter>
      </QueryClientProvider>,
    );
    expect(screen.queryByTestId("board-new-project")).toBeNull();
  });

  it("opens the new-project modal on click", () => {
    renderBoard();
    expect(screen.queryByTestId("new-project-modal")).toBeNull();

    fireEvent.click(screen.getByTestId("board-new-project"));

    expect(screen.getByTestId("new-project-modal")).toBeDefined();
  });
});

describe("IssuesBoard column drag-and-drop reordering", () => {
  beforeEach(() => {
    projectsData = [
      makeProject("j-eng", "engineering", ENG_STATUSES, "Engineering"),
    ];
  });

  it("registers a SortableContext over the project's status keys", () => {
    renderBoard();
    expect(lastSortableItems).toEqual(["open", "in-progress"]);
  });

  it("renders the column head with draggable handle attributes", () => {
    renderBoard();
    const head = screen.getByTestId("board-col-head-engineering-open");
    // The mocked useSortable returns a marker attribute the real component
    // spreads onto the head; assert it lands there so the drag-handle wiring
    // is actually plumbed through.
    expect(head.getAttribute("data-sortable-handle")).toBe("true");
  });

  it("on drop reorders statuses and calls apiClient.updateProject with the new order", async () => {
    renderBoard();
    expect(lastDragEndHandler).not.toBeNull();

    act(() => {
      lastDragEndHandler!({
        active: { id: "open" },
        over: { id: "in-progress" },
      });
    });

    await waitFor(() => expect(mockUpdateProject).toHaveBeenCalledTimes(1));
    const [projectId, body] = mockUpdateProject.mock.calls[0];
    expect(projectId).toBe("j-eng");
    const keys = (body.project.statuses as StatusDefinition[]).map((s) => s.key);
    expect(keys).toEqual(["in-progress", "open"]);
    // Per-status fields must be preserved during reorder.
    const moved = (body.project.statuses as StatusDefinition[]).find(
      (s) => s.key === "in-progress",
    );
    expect(moved?.label).toBe("In progress");
    expect(moved?.color).toBe("#f1c40f");
  });

  it("is a no-op when the drop target equals the dragged item", () => {
    renderBoard();
    act(() => {
      lastDragEndHandler!({
        active: { id: "open" },
        over: { id: "open" },
      });
    });
    expect(mockUpdateProject).not.toHaveBeenCalled();
  });

  it("optimistically reorders the projects cache and rolls back on save error", async () => {
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
    });
    const before: ListProjectsResponse = { projects: [...projectsData!] };
    client.setQueryData(["projects"], before);

    // Hold the mutation in-flight so we can observe the optimistic write
    // before the failure resolves it back to the previous snapshot.
    let rejectUpdate: ((err: Error) => void) | null = null;
    mockUpdateProject.mockReturnValueOnce(
      new Promise<never>((_, reject) => {
        rejectUpdate = reject;
      }),
    );

    renderBoard(client);

    act(() => {
      lastDragEndHandler!({
        active: { id: "open" },
        over: { id: "in-progress" },
      });
    });

    // Optimistic write is visible while the mutation is still pending.
    await waitFor(() => {
      const snapshot = client.getQueryData<ListProjectsResponse>(["projects"]);
      const keys = snapshot?.projects[0]?.project.statuses.map((s) => s.key);
      expect(keys).toEqual(["in-progress", "open"]);
    });

    act(() => {
      rejectUpdate!(new Error("boom"));
    });

    // Once the mutation rejects, onError restores the prior snapshot.
    await waitFor(() => {
      const snapshot = client.getQueryData<ListProjectsResponse>(["projects"]);
      const keys = snapshot?.projects[0]?.project.statuses.map((s) => s.key);
      expect(keys).toEqual(["open", "in-progress"]);
    });
    expect(mockAddToast).toHaveBeenCalledWith("boom", "error");
  });

  it("does not call updateProject when there are no projects", () => {
    projectsData = [];
    renderBoard();
    // No project sections means no DndContext was mounted.
    expect(lastDragEndHandler).toBeNull();
    expect(mockUpdateProject).not.toHaveBeenCalled();
  });
});

describe("IssuesBoard hideIssues (Projects tab)", () => {
  beforeEach(() => {
    projectsData = [
      makeProject("j-eng", "engineering", ENG_STATUSES, "Engineering"),
    ];
  });

  it("keeps the project sections, status columns, and add-status ghost rendered", () => {
    renderBoard(undefined, { hideIssues: true });

    // Project sections still render.
    expect(screen.getByTestId("board-project-engineering")).toBeDefined();
    // Status columns still render.
    expect(screen.getByTestId("board-col-engineering-open")).toBeDefined();
    expect(screen.getByTestId("board-col-engineering-in-progress")).toBeDefined();
    // The "+ Add status" and "+ New project" ghosts remain.
    expect(screen.getByTestId("board-col-add-engineering")).toBeDefined();
    expect(screen.getByTestId("board-new-project")).toBeDefined();
  });

  it("suppresses the per-column 'No issues' placeholder", () => {
    renderBoard(undefined, { hideIssues: true });

    expect(screen.queryByText("No issues")).toBeNull();
    expect(screen.queryByText("Loading…")).toBeNull();
  });

  it("suppresses the per-project 'N issues' meta pill", () => {
    renderBoard(undefined, { hideIssues: true });

    // The board normally renders e.g. "0 issues" next to the project chip.
    // In projects-only mode that pill should not appear.
    expect(screen.queryByText(/^\d+ issues?$/)).toBeNull();
    // The "N statuses" pill should still render.
    expect(screen.getByText(/^2 statuses$/)).toBeDefined();
  });

  it("renders columns in the same order as the normal board (status chips visible)", () => {
    renderBoard(undefined, { hideIssues: true });

    // Status chips are emitted via StatusChip; the column heads carry
    // testids parameterised on the status key — verify both eng statuses
    // render in order.
    const cols = [
      screen.getByTestId("board-col-engineering-open"),
      screen.getByTestId("board-col-engineering-in-progress"),
    ];
    // Each column head must still be present (chrome match parity).
    expect(within(cols[0]).getByTestId("board-col-head-engineering-open")).toBeDefined();
    expect(
      within(cols[1]).getByTestId("board-col-head-engineering-in-progress"),
    ).toBeDefined();
  });
});

describe("IssuesBoard project drag-and-drop reordering", () => {
  function makeProjectWithPriority(
    id: string,
    key: string,
    priority: number,
  ): ProjectRecord {
    const rec = makeProject(id, key, DEFAULT_STATUSES, key);
    rec.project.priority = priority;
    return rec;
  }

  beforeEach(() => {
    projectsData = [
      makeProjectWithPriority("j-a", "alpha", 1000),
      makeProjectWithPriority("j-b", "beta", 2000),
      makeProjectWithPriority("j-c", "gamma", 3000),
    ];
  });

  // The board renders the project-reorder DndContext first, before each
  // ProjectSection's status-reorder DndContext. That puts the project
  // handler at index 0 of `dragEndHandlers`.
  function projectDragEndHandler(): (event: unknown) => void {
    expect(dragEndHandlers.length).toBeGreaterThan(0);
    return dragEndHandlers[0]!;
  }

  it("registers a SortableContext over project ids when there's more than one project", () => {
    renderBoard();
    // First-rendered SortableContext is the project-level one. Status-level
    // contexts are rendered later by each ProjectSection.
    expect(sortableItemsList[0]).toEqual(["j-a", "j-b", "j-c"]);
  });

  it("decorates the project bar with the drag-handle marker when reorder is allowed", () => {
    renderBoard();
    expect(
      screen.getByTestId("board-project-bar-alpha").getAttribute(
        "data-sortable-handle",
      ),
    ).toBe("true");
  });

  it("skips the project DndContext when scoped to a single project", () => {
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
    });
    render(
      <QueryClientProvider client={client}>
        <MemoryRouter>
          <IssuesBoard
            baseFilters={{ project_id: "j-a" }}
            username="alice"
            filterRootId={null}
          />
        </MemoryRouter>
      </QueryClientProvider>,
    );
    // Only the status-level SortableContext should be registered.
    expect(sortableItemsList).toHaveLength(1);
    expect(sortableItemsList[0]).toEqual(["open"]);
    // The single project's bar shouldn't be a drag handle.
    expect(
      screen.getByTestId("board-project-bar-alpha").getAttribute(
        "data-sortable-handle",
      ),
    ).toBeNull();
  });

  it("skips the project DndContext when only one project exists", () => {
    projectsData = [makeProjectWithPriority("j-a", "alpha", 1000)];
    renderBoard();
    // Only the status-level SortableContext should be registered.
    expect(sortableItemsList).toHaveLength(1);
    expect(sortableItemsList[0]).toEqual(["open"]);
    expect(
      screen.getByTestId("board-project-bar-alpha").getAttribute(
        "data-sortable-handle",
      ),
    ).toBeNull();
  });

  it("on drop between two neighbors sets priority to their midpoint", async () => {
    renderBoard();
    // Move 'gamma' (3000) to where 'alpha' (1000) was — between alpha and
    // beta in the new order. New neighbors after move: alpha (1000) and
    // beta (2000). Midpoint = 1500.
    act(() => {
      projectDragEndHandler()({
        active: { id: "j-c" },
        over: { id: "j-b" },
      });
    });

    await waitFor(() => expect(mockUpdateProject).toHaveBeenCalledTimes(1));
    const [projectId, body] = mockUpdateProject.mock.calls[0];
    expect(projectId).toBe("j-c");
    expect(body.project.priority).toBe(1500);
    // Other project fields should be preserved.
    expect(body.project.key).toBe("gamma");
  });

  it("on drop at the top sets priority below the first neighbor's", async () => {
    renderBoard();
    // Drop gamma onto alpha — gamma ends up first, alpha shifts down.
    act(() => {
      projectDragEndHandler()({
        active: { id: "j-c" },
        over: { id: "j-a" },
      });
    });

    await waitFor(() => expect(mockUpdateProject).toHaveBeenCalledTimes(1));
    const [, body] = mockUpdateProject.mock.calls[0];
    // alpha's priority is 1000; gamma at top = 1000 - 1024 = -24.
    expect(body.project.priority).toBe(-24);
  });

  it("on drop at the bottom sets priority above the last neighbor's", async () => {
    renderBoard();
    // Drop alpha onto gamma — alpha ends up last.
    act(() => {
      projectDragEndHandler()({
        active: { id: "j-a" },
        over: { id: "j-c" },
      });
    });

    await waitFor(() => expect(mockUpdateProject).toHaveBeenCalledTimes(1));
    const [, body] = mockUpdateProject.mock.calls[0];
    // gamma's priority is 3000; alpha at bottom = 3000 + 1024 = 4024.
    expect(body.project.priority).toBe(4024);
  });

  it("is a no-op when the drop target equals the dragged project", () => {
    renderBoard();
    act(() => {
      projectDragEndHandler()({
        active: { id: "j-a" },
        over: { id: "j-a" },
      });
    });
    expect(mockUpdateProject).not.toHaveBeenCalled();
  });

  it("optimistically reorders the projects cache and rolls back on save error", async () => {
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
    });
    const before: ListProjectsResponse = { projects: [...projectsData!] };
    client.setQueryData(["projects"], before);

    let rejectUpdate: ((err: Error) => void) | null = null;
    mockUpdateProject.mockReturnValueOnce(
      new Promise<never>((_, reject) => {
        rejectUpdate = reject;
      }),
    );

    renderBoard(client);

    // Move gamma to between alpha and beta — priority becomes 1500,
    // sorting it into the middle position.
    act(() => {
      projectDragEndHandler()({
        active: { id: "j-c" },
        over: { id: "j-b" },
      });
    });

    await waitFor(() => {
      const snapshot = client.getQueryData<ListProjectsResponse>(["projects"]);
      const ids = snapshot?.projects.map((p) => p.project_id);
      expect(ids).toEqual(["j-a", "j-c", "j-b"]);
      const gamma = snapshot?.projects.find((p) => p.project_id === "j-c");
      expect(gamma?.project.priority).toBe(1500);
    });

    act(() => {
      rejectUpdate!(new Error("nope"));
    });

    await waitFor(() => {
      const snapshot = client.getQueryData<ListProjectsResponse>(["projects"]);
      const ids = snapshot?.projects.map((p) => p.project_id);
      expect(ids).toEqual(["j-a", "j-b", "j-c"]);
    });
    expect(mockAddToast).toHaveBeenCalledWith("nope", "error");
  });
});
