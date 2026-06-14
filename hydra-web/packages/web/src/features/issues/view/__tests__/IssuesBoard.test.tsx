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
  ConversationSummary,
  Issue,
  IssueSummary,
  IssueSummaryRecord,
  ListIssuesResponse,
  ListProjectsResponse,
  ProjectRecord,
  StatusDefinition,
  UpsertIssueRequest,
} from "@hydra/api";
import type { BoardCellQuery } from "../../usePaginatedIssues";

// --- Hook mocks ---

let projectsData: ProjectRecord[] | undefined = [];
let cellsByProject: Map<string, Map<string, BoardCellQuery>> = new Map();

// Force desktop branch by default. Tests that need mobile override this.
let mobileMatches = false;
vi.mock("../../../../hooks/useMediaQuery", () => ({
  useMediaQuery: () => mobileMatches,
}));

vi.mock("../../../projects/useProjects", () => ({
  useProjects: () => ({ data: projectsData }),
}));

vi.mock("../../usePaginatedIssues", () => ({
  useBoardIssuesByProject: () => cellsByProject,
  // Must mirror the live export — IssuesBoard's optimistic drag-drop reads
  // the query-cache shape via this marker, and `vi.mock` would otherwise
  // leave it undefined.
  BOARD_BULK_QUERY_KEY_MARKER: "board-bulk",
}));

vi.mock("../../../dashboard/usePageIssueTrees", () => ({
  usePageIssueTrees: () => ({
    neighborhoodMap: new Map(),
    sessionsByIssue: new Map(),
    isLoading: false,
  }),
}));

let conversationsByIssueMap: Map<string, ConversationSummary> = new Map();
vi.mock("../../../chat/useActiveConversationsByIssue", () => ({
  useActiveConversationsByIssue: () => conversationsByIssueMap,
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
  DragOverlay: ({ children }: { children?: React.ReactNode }) => <>{children}</>,
  PointerSensor: function PointerSensor() {},
  MouseSensor: function MouseSensor() {},
  TouchSensor: function TouchSensor() {},
  KeyboardSensor: function KeyboardSensor() {},
  useSensor: () => ({}),
  useSensors: () => [],
  closestCenter: () => [],
  MeasuringStrategy: {
    Always: "always",
    BeforeDragging: "before-dragging",
    WhileDragging: "while-dragging",
  },
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
const mockUpdateIssue = vi.fn();
const mockGetIssue = vi.fn();
const mockUpdateProjectStatus = vi.fn(
  async (
    projectId: string,
    _statusKey: string,
    status: StatusDefinition,
  ): Promise<{
    project_id: string;
    version: number;
    status: StatusDefinition;
  }> => ({
    project_id: projectId,
    version: 1,
    status,
  }),
);
vi.mock("../../../../api/client", () => ({
  apiClient: {
    updateProject: (
      projectId: string,
      request: {
        key: string;
        name: string;
        prompt_path: string | null;
        priority: number;
      },
    ) => mockUpdateProject(projectId, request),
    updateIssue: (issueId: string, request: UpsertIssueRequest) =>
      mockUpdateIssue(issueId, request),
    getIssue: (issueId: string) => mockGetIssue(issueId),
    updateProjectStatus: (
      projectId: string,
      statusKey: string,
      status: StatusDefinition,
    ) => mockUpdateProjectStatus(projectId, statusKey, status),
  },
}));

const mockAddToast = vi.fn();
vi.mock("../../../toast/useToast", () => ({
  useToast: () => ({ addToast: mockAddToast }),
}));

const mockOpenIssueCreate = vi.fn();
vi.mock("../../../dashboard/useIssueCreateModal", () => ({
  useIssueCreateModal: () => ({
    isOpen: false,
    initial: null,
    open: mockOpenIssueCreate,
    close: vi.fn(),
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
  FlowPill: () => <span data-testid="flow-pill" />,
  Button: ({
    children,
    ...props
  }: React.ButtonHTMLAttributes<HTMLButtonElement> & {
    variant?: string;
    size?: string;
  }) => {
    const { variant: _variant, size: _size, ...rest } = props;
    return <button {...rest}>{children}</button>;
  },
  Picker: ({
    label,
    open,
    onToggle,
    value,
    children,
    "data-testid": testId,
  }: {
    label: string;
    open: boolean;
    onToggle: () => void;
    value: React.ReactNode;
    children: React.ReactNode;
    "data-testid"?: string;
  }) => (
    <div data-testid={testId}>
      <button type="button" aria-label={label} onClick={onToggle}>
        {value}
      </button>
      {open ? <div>{children}</div> : null}
    </div>
  ),
  PickerRow: ({
    active,
    onClick,
    children,
    "data-testid": testId,
  }: {
    active?: boolean;
    onClick: () => void;
    children: React.ReactNode;
    "data-testid"?: string;
  }) => (
    <button
      type="button"
      data-testid={testId}
      data-active={active ? "true" : undefined}
      onClick={onClick}
    >
      {children}
    </button>
  ),
  Icons: {
    IconSettings: () => <span data-testid="icon-settings" />,
    IconSpark: () => <span data-testid="icon-spark" />,
    IconChevronDown: () => <span data-testid="icon-chevron-down" />,
    IconPlus: () => <span data-testid="icon-plus" />,
    IconChat: () => <span data-testid="icon-chat" />,
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

// Replace ProjectForm with a sentinel — the settings (edit) modal renders it.
// The new-project modal also uses it, but that route is mocked separately
// below.
vi.mock("../../../projects/ProjectForm", () => ({
  ProjectForm: ({
    projectId,
    initial,
    creator,
  }: {
    projectId?: string | null;
    initial?: { key: string; name: string };
    creator: string;
  }) => (
    <div
      data-testid="project-form"
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
    position: 0,
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
      archived: false,
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
          filterRootId={null}
          hideIssues={opts.hideIssues}
        />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  mobileMatches = false;
  projectsData = [];
  cellsByProject = new Map();
  conversationsByIssueMap = new Map();
  lastModalProps.open = undefined;
  lastModalProps.projectRecord = undefined;
  lastModalProps.statusKey = undefined;
  lastModalProps.issueCount = undefined;
  lastDragEndHandler = null;
  lastSortableItems = [];
  dragEndHandlers = [];
  sortableItemsList = [];
  mockUpdateProject.mockReset();
  mockUpdateProjectStatus.mockClear();
  mockUpdateProjectStatus.mockImplementation(
    async (
      projectId: string,
      _statusKey: string,
      status: StatusDefinition,
    ) => ({
      project_id: projectId,
      version: 1,
      status,
    }),
  );
  mockUpdateProject.mockResolvedValue({ project_id: "j-eng", version: 2 });
  mockUpdateIssue.mockReset();
  mockUpdateIssue.mockResolvedValue({ issue_id: "i-x", version: 2 });
  mockGetIssue.mockReset();
  mockAddToast.mockReset();
  mockOpenIssueCreate.mockReset();
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

  it("renders 'interactive' badge with the Spark icon and the interactive modifier class when status.interactive === true", () => {
    const status = makeStatus({
      key: "triage",
      label: "Triage",
      interactive: true,
    });
    projectsData = [makeProject("j-proj", "proj", [status])];

    renderBoard();

    const mode = screen.getByTestId("board-col-mode-proj-triage");
    expect(mode.textContent).toBe("interactive");
    expect(within(mode).getByTestId("icon-spark")).toBeTruthy();
    expect(mode.className).toContain("modeBadgeInteractive");
    expect(mode.getAttribute("title")).toBe(
      "Interactive — human in the loop",
    );
  });

  it("renders 'auto' badge without the interactive modifier or Spark icon", () => {
    const status = makeStatus({ key: "open", label: "Open" });
    projectsData = [makeProject("j-proj", "proj", [status])];

    renderBoard();

    const mode = screen.getByTestId("board-col-mode-proj-open");
    expect(mode.className).not.toContain("modeBadgeInteractive");
    expect(within(mode).queryByTestId("icon-spark")).toBeNull();
    expect(mode.getAttribute("title")).toBe("Autonomous agent work");
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
    const editor = screen.getByTestId("project-form");
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

describe("IssuesBoard '+ Add issue' per-column button", () => {
  beforeEach(() => {
    projectsData = [
      makeProject("j-eng", "engineering", ENG_STATUSES, "Engineering"),
    ];
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

  it("renders an '+ Add issue' button inside each status column", () => {
    renderBoard();

    expect(
      screen.getByTestId("board-col-add-issue-engineering-open"),
    ).toBeDefined();
    expect(
      screen.getByTestId("board-col-add-issue-engineering-in-progress"),
    ).toBeDefined();
  });

  it("opens the new-issue modal with the column's project and status prepopulated", () => {
    renderBoard();

    fireEvent.click(
      screen.getByTestId("board-col-add-issue-engineering-in-progress"),
    );

    expect(mockOpenIssueCreate).toHaveBeenCalledTimes(1);
    expect(mockOpenIssueCreate).toHaveBeenCalledWith({
      projectId: "j-eng",
      status: "in-progress",
    });
  });

  it("never renders a 'No issues' placeholder, even with empty cells", () => {
    renderBoard();

    expect(screen.queryByText("No issues")).toBeNull();
  });

  it("is not rendered in hideIssues mode (Projects tab)", () => {
    renderBoard(undefined, { hideIssues: true });

    expect(
      screen.queryByTestId("board-col-add-issue-engineering-open"),
    ).toBeNull();
    expect(
      screen.queryByTestId("board-col-add-issue-engineering-in-progress"),
    ).toBeNull();
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

  it("on drop reorders statuses and calls updateProjectStatus with new positions", async () => {
    renderBoard();
    expect(lastDragEndHandler).not.toBeNull();

    act(() => {
      lastDragEndHandler!({
        active: { id: "open" },
        over: { id: "in-progress" },
      });
    });

    await waitFor(() => expect(mockUpdateProjectStatus).toHaveBeenCalledTimes(2));
    const calls = mockUpdateProjectStatus.mock.calls.map((c) => ({
      projectId: c[0] as string,
      statusKey: c[1] as string,
      position: (c[2] as StatusDefinition).position,
    }));
    expect(calls).toEqual([
      { projectId: "j-eng", statusKey: "in-progress", position: 0 },
      { projectId: "j-eng", statusKey: "open", position: 100 },
    ]);
  });

  it("is a no-op when the drop target equals the dragged item", () => {
    renderBoard();
    act(() => {
      lastDragEndHandler!({
        active: { id: "open" },
        over: { id: "open" },
      });
    });
    expect(mockUpdateProjectStatus).not.toHaveBeenCalled();
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
    mockUpdateProjectStatus.mockReturnValueOnce(
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

  it("does not call updateProjectStatus when there are no projects", () => {
    projectsData = [];
    renderBoard();
    // No project sections means no DndContext was mounted.
    expect(lastDragEndHandler).toBeNull();
    expect(mockUpdateProjectStatus).not.toHaveBeenCalled();
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

  it("suppresses the per-column 'Loading…' placeholder", () => {
    renderBoard(undefined, { hideIssues: true });

    // The "No issues" placeholder was removed unconditionally — see the
    // "'+ Add issue' per-column button" describe block. Here we only assert
    // that hideIssues continues to suppress the transient loading state.
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

describe("IssuesBoard project collapse chevron", () => {
  beforeEach(() => {
    projectsData = [
      makeProject("j-eng", "engineering", ENG_STATUSES, "Engineering"),
      makeProject("j-design", "design", DEFAULT_STATUSES, "Design"),
    ];
    window.localStorage.clear();
  });

  afterEach(() => {
    window.localStorage.clear();
  });

  it("renders a collapse chevron on every project bar, expanded by default", () => {
    renderBoard();

    const engChevron = screen.getByTestId("board-project-collapse-engineering");
    const designChevron = screen.getByTestId("board-project-collapse-design");
    expect(engChevron.getAttribute("aria-expanded")).toBe("true");
    expect(designChevron.getAttribute("aria-expanded")).toBe("true");
    // Body wrappers exist in the expanded state.
    expect(screen.getByTestId("board-project-body-engineering")).toBeDefined();
    expect(screen.getByTestId("board-project-body-design")).toBeDefined();
  });

  it("clicking the chevron collapses only that project's body", () => {
    renderBoard();
    fireEvent.click(screen.getByTestId("board-project-collapse-engineering"));

    const engChevron = screen.getByTestId("board-project-collapse-engineering");
    expect(engChevron.getAttribute("aria-expanded")).toBe("false");
    expect(engChevron.className).toContain("projectCollapseToggleCollapsed");

    const engBody = screen.getByTestId("board-project-body-engineering");
    expect(engBody.className).toContain("projectGroupBodyCollapsed");
    expect(engBody.getAttribute("aria-hidden")).toBe("true");

    // The other project stays expanded.
    const designBody = screen.getByTestId("board-project-body-design");
    expect(designBody.className).not.toContain("projectGroupBodyCollapsed");
    expect(designBody.getAttribute("aria-hidden")).toBe("false");
  });

  it("clicking the chevron a second time re-expands the project", () => {
    renderBoard();
    fireEvent.click(screen.getByTestId("board-project-collapse-engineering"));
    fireEvent.click(screen.getByTestId("board-project-collapse-engineering"));

    const engChevron = screen.getByTestId("board-project-collapse-engineering");
    expect(engChevron.getAttribute("aria-expanded")).toBe("true");
    const engBody = screen.getByTestId("board-project-body-engineering");
    expect(engBody.className).not.toContain("projectGroupBodyCollapsed");
  });

  it("persists the collapsed project ids in localStorage", () => {
    renderBoard();
    fireEvent.click(screen.getByTestId("board-project-collapse-engineering"));

    const raw = window.localStorage.getItem("hydra:board-project-collapsed");
    expect(raw).not.toBeNull();
    expect(JSON.parse(raw!)).toEqual(["j-eng"]);
  });

  it("rehydrates the collapsed set from localStorage on mount", () => {
    window.localStorage.setItem(
      "hydra:board-project-collapsed",
      JSON.stringify(["j-design"]),
    );
    renderBoard();

    expect(
      screen
        .getByTestId("board-project-collapse-design")
        .getAttribute("aria-expanded"),
    ).toBe("false");
    expect(
      screen.getByTestId("board-project-body-design").className,
    ).toContain("projectGroupBodyCollapsed");
    // engineering stays expanded.
    expect(
      screen
        .getByTestId("board-project-collapse-engineering")
        .getAttribute("aria-expanded"),
    ).toBe("true");
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
    expect(body.priority).toBe(1500);
    // Other project fields should be preserved.
    expect(body.key).toBe("gamma");
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
    expect(body.priority).toBe(-24);
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
    expect(body.priority).toBe(4024);
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

describe("IssuesBoard issue-card drag-and-drop", () => {
  function makeSummaryRecord(
    issueId: string,
    title: string,
    projectId: string,
    statusKey: string,
  ): IssueSummaryRecord {
    const status: StatusDefinition = makeStatus({
      key: statusKey,
      label: statusKey,
    });
    const summary: IssueSummary = {
      type: "task",
      title,
      description: "",
      creator: "alice",
      status,
      project_id: projectId,
      dependencies: [],
      patches: [],
    };
    return {
      issue_id: issueId,
      version: BigInt(1),
      timestamp: "2026-06-09T00:00:00Z",
      issue: summary,
      creation_time: "2026-06-09T00:00:00Z",
    };
  }

  function makeFullIssue(
    projectId: string,
    statusKey: string,
    extra?: Partial<Issue>,
  ): Issue {
    return {
      type: "task",
      title: "Card title",
      description: "Full description, possibly long",
      creator: "alice",
      status: makeStatus({ key: statusKey, label: statusKey }),
      project_id: projectId,
      dependencies: [],
      patches: [],
      ...extra,
    };
  }

  // jsdom does not implement DataTransfer; we wire a minimal stand-in that
  // supports the setData/getData/types subset the production code uses.
  function makeDataTransfer() {
    const store: Record<string, string> = {};
    return {
      effectAllowed: "all" as DataTransfer["effectAllowed"],
      dropEffect: "none" as DataTransfer["dropEffect"],
      types: [] as string[],
      setData(format: string, value: string) {
        store[format] = value;
        if (!this.types.includes(format)) this.types.push(format);
      },
      getData(format: string) {
        return store[format] ?? "";
      },
    };
  }

  const ENG_PROJECT = makeProject("j-eng", "engineering", ENG_STATUSES, "Eng");
  const PRO_PROJECT = makeProject("j-pro", "product", ENG_STATUSES, "Product");

  beforeEach(() => {
    projectsData = [ENG_PROJECT];
    cellsByProject = new Map();
  });

  it("renders draggable issue cards in each column", () => {
    const rec = makeSummaryRecord("i-aaa", "first", "j-eng", "open");
    cellsByProject = new Map([
      [
        "j-eng",
        new Map<string, BoardCellQuery>([
          ["open", emptyCell({ issues: [rec] })],
          ["in-progress", emptyCell()],
        ]),
      ],
    ]);
    renderBoard();

    const card = screen.getByTestId("board-card-i-aaa");
    expect(card.getAttribute("draggable")).toBe("true");
  });

  it("on drop to a different column, calls updateIssue with the new status", async () => {
    const rec = makeSummaryRecord("i-aaa", "first", "j-eng", "open");
    cellsByProject = new Map([
      [
        "j-eng",
        new Map<string, BoardCellQuery>([
          ["open", emptyCell({ issues: [rec] })],
          ["in-progress", emptyCell()],
        ]),
      ],
    ]);
    mockGetIssue.mockResolvedValueOnce({
      issue_id: "i-aaa",
      version: BigInt(1),
      timestamp: "t",
      issue: makeFullIssue("j-eng", "open"),
      creation_time: "t",
    });

    renderBoard();
    const card = screen.getByTestId("board-card-i-aaa");
    const target = screen.getByTestId("board-col-engineering-in-progress");

    const dt = makeDataTransfer();
    fireEvent.dragStart(card, { dataTransfer: dt });
    fireEvent.dragOver(target, { dataTransfer: dt });
    fireEvent.drop(target, { dataTransfer: dt });

    await waitFor(() => expect(mockUpdateIssue).toHaveBeenCalledTimes(1));
    const [issueId, request] = mockUpdateIssue.mock.calls[0];
    expect(issueId).toBe("i-aaa");
    // The mutation fetches the full Issue (so we don't clobber the
    // description with the summary's truncation) then updates with new
    // status + project_id.
    expect(mockGetIssue).toHaveBeenCalledWith("i-aaa");
    expect(request.issue.status).toBe("in-progress");
    expect(request.issue.project_id).toBe("j-eng");
    // The fetched description (not the truncated summary) is preserved.
    expect(request.issue.description).toBe("Full description, possibly long");
    expect(request.session_id).toBeNull();
  });

  it("on drop to a different project's column, sets both project_id and status", async () => {
    projectsData = [ENG_PROJECT, PRO_PROJECT];
    const rec = makeSummaryRecord("i-aaa", "first", "j-eng", "open");
    cellsByProject = new Map([
      [
        "j-eng",
        new Map<string, BoardCellQuery>([
          ["open", emptyCell({ issues: [rec] })],
          ["in-progress", emptyCell()],
        ]),
      ],
      [
        "j-pro",
        new Map<string, BoardCellQuery>([
          ["open", emptyCell()],
          ["in-progress", emptyCell()],
        ]),
      ],
    ]);
    mockGetIssue.mockResolvedValueOnce({
      issue_id: "i-aaa",
      version: BigInt(1),
      timestamp: "t",
      issue: makeFullIssue("j-eng", "open"),
      creation_time: "t",
    });

    renderBoard();
    const card = screen.getByTestId("board-card-i-aaa");
    const target = screen.getByTestId("board-col-product-in-progress");

    const dt = makeDataTransfer();
    fireEvent.dragStart(card, { dataTransfer: dt });
    fireEvent.dragOver(target, { dataTransfer: dt });
    fireEvent.drop(target, { dataTransfer: dt });

    await waitFor(() => expect(mockUpdateIssue).toHaveBeenCalledTimes(1));
    const [issueId, request] = mockUpdateIssue.mock.calls[0];
    expect(issueId).toBe("i-aaa");
    expect(request.issue.project_id).toBe("j-pro");
    expect(request.issue.status).toBe("in-progress");
  });

  it("is a no-op when the card is dropped on its source column", () => {
    const rec = makeSummaryRecord("i-aaa", "first", "j-eng", "open");
    cellsByProject = new Map([
      [
        "j-eng",
        new Map<string, BoardCellQuery>([
          ["open", emptyCell({ issues: [rec] })],
          ["in-progress", emptyCell()],
        ]),
      ],
    ]);

    renderBoard();
    const card = screen.getByTestId("board-card-i-aaa");
    const source = screen.getByTestId("board-col-engineering-open");

    const dt = makeDataTransfer();
    fireEvent.dragStart(card, { dataTransfer: dt });
    fireEvent.dragOver(source, { dataTransfer: dt });
    fireEvent.drop(source, { dataTransfer: dt });

    expect(mockUpdateIssue).not.toHaveBeenCalled();
    expect(mockGetIssue).not.toHaveBeenCalled();
  });

  it("optimistically moves the card between cached cells and rolls back on error", async () => {
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
    });
    const rec = makeSummaryRecord("i-aaa", "first", "j-eng", "open");
    cellsByProject = new Map([
      [
        "j-eng",
        new Map<string, BoardCellQuery>([
          ["open", emptyCell({ issues: [rec] })],
          ["in-progress", emptyCell()],
        ]),
      ],
    ]);

    // Pre-seed the paginatedIssues cache with the source and target cells.
    // The hook's queryKey shape is
    //   ["paginatedIssues", filtersForKey, "depth", depth]
    // where filtersForKey = { ...baseFilters, project_id, status }. The
    // board is rendered with empty baseFilters so the only filter fields
    // are project_id and status.
    const sourceKey = [
      "paginatedIssues",
      { project_id: "j-eng", status: "open" },
      "depth",
      1,
    ] as const;
    const targetKey = [
      "paginatedIssues",
      { project_id: "j-eng", status: "in-progress" },
      "depth",
      1,
    ] as const;
    const sourcePages: ListIssuesResponse[] = [
      { issues: [rec], next_cursor: null },
    ];
    const targetPages: ListIssuesResponse[] = [{ issues: [], next_cursor: null }];
    client.setQueryData(sourceKey, sourcePages);
    client.setQueryData(targetKey, targetPages);

    mockGetIssue.mockResolvedValueOnce({
      issue_id: "i-aaa",
      version: BigInt(1),
      timestamp: "t",
      issue: makeFullIssue("j-eng", "open"),
      creation_time: "t",
    });
    let rejectUpdate: ((err: Error) => void) | null = null;
    mockUpdateIssue.mockReturnValueOnce(
      new Promise<never>((_, reject) => {
        rejectUpdate = reject;
      }),
    );

    renderBoard(client);
    const card = screen.getByTestId("board-card-i-aaa");
    const target = screen.getByTestId("board-col-engineering-in-progress");
    const dt = makeDataTransfer();
    fireEvent.dragStart(card, { dataTransfer: dt });
    fireEvent.dragOver(target, { dataTransfer: dt });
    fireEvent.drop(target, { dataTransfer: dt });

    // Optimistic write: the issue is removed from the source cell and added
    // to the target cell while the mutation is still pending.
    await waitFor(() => {
      const src = client.getQueryData<ListIssuesResponse[]>(sourceKey);
      const tgt = client.getQueryData<ListIssuesResponse[]>(targetKey);
      expect(src?.[0].issues.map((r) => r.issue_id)).toEqual([]);
      expect(tgt?.[0].issues.map((r) => r.issue_id)).toEqual(["i-aaa"]);
      expect(tgt?.[0].issues[0].issue.status.key).toBe("in-progress");
    });

    act(() => {
      rejectUpdate!(new Error("boom"));
    });

    // Rollback restores the original cache snapshots.
    await waitFor(() => {
      const src = client.getQueryData<ListIssuesResponse[]>(sourceKey);
      const tgt = client.getQueryData<ListIssuesResponse[]>(targetKey);
      expect(src?.[0].issues.map((r) => r.issue_id)).toEqual(["i-aaa"]);
      expect(tgt?.[0].issues).toEqual([]);
    });
    expect(mockAddToast).toHaveBeenCalledWith("boom", "error");
  });

  // Regression: the table-view `usePaginatedIssues` hook is a
  // `useInfiniteQuery` whose `data` is `{ pages, pageParams }`, sharing the
  // `["paginatedIssues", …]` prefix with the board cells. The optimistic
  // cache walk must skip those entries — iterating their data as an array
  // throws `data is not iterable` and the mutation never fires.
  it("ignores cached infinite-query entries that share the paginatedIssues prefix", async () => {
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
    });
    const rec = makeSummaryRecord("i-aaa", "first", "j-eng", "open");
    cellsByProject = new Map([
      [
        "j-eng",
        new Map<string, BoardCellQuery>([
          ["open", emptyCell({ issues: [rec] })],
          ["in-progress", emptyCell()],
        ]),
      ],
    ]);

    // Seed the table-view (2-element key) entry FIRST so it precedes the
    // board-cell entries in the cache iteration order — this is the order
    // the source-record search loop would naturally encounter when a user
    // visits the issues page (table is the default layout) and then
    // switches to board.
    const tableKey = ["paginatedIssues", {}] as const;
    client.setQueryData(tableKey, {
      pages: [{ issues: [rec], next_cursor: null }],
      pageParams: [undefined],
    });

    // Board-cell (4-element key with "depth") entries.
    const sourceKey = [
      "paginatedIssues",
      { project_id: "j-eng", status: "open" },
      "depth",
      1,
    ] as const;
    const targetKey = [
      "paginatedIssues",
      { project_id: "j-eng", status: "in-progress" },
      "depth",
      1,
    ] as const;
    client.setQueryData<ListIssuesResponse[]>(sourceKey, [
      { issues: [rec], next_cursor: null },
    ]);
    client.setQueryData<ListIssuesResponse[]>(targetKey, [
      { issues: [], next_cursor: null },
    ]);

    mockGetIssue.mockResolvedValueOnce({
      issue_id: "i-aaa",
      version: BigInt(1),
      timestamp: "t",
      issue: makeFullIssue("j-eng", "open"),
      creation_time: "t",
    });

    renderBoard(client);
    const card = screen.getByTestId("board-card-i-aaa");
    const target = screen.getByTestId("board-col-engineering-in-progress");
    const dt = makeDataTransfer();
    fireEvent.dragStart(card, { dataTransfer: dt });
    fireEvent.dragOver(target, { dataTransfer: dt });
    fireEvent.drop(target, { dataTransfer: dt });

    // Mutation fires and the board cells are optimistically updated.
    await waitFor(() => expect(mockUpdateIssue).toHaveBeenCalledTimes(1));
    await waitFor(() => {
      const src = client.getQueryData<ListIssuesResponse[]>(sourceKey);
      const tgt = client.getQueryData<ListIssuesResponse[]>(targetKey);
      expect(src?.[0].issues.map((r) => r.issue_id)).toEqual([]);
      expect(tgt?.[0].issues.map((r) => r.issue_id)).toEqual(["i-aaa"]);
    });
    // No error toast — the infinite-query entry was skipped, not crashed on.
    expect(mockAddToast).not.toHaveBeenCalled();
    // The infinite-query entry itself was left untouched by the optimistic
    // surgery (invalidation in onSettled is what refreshes it).
    const tableSnapshot = client.getQueryData<{
      pages: ListIssuesResponse[];
      pageParams: unknown[];
    }>(tableKey);
    expect(tableSnapshot?.pages[0].issues.map((r) => r.issue_id)).toEqual([
      "i-aaa",
    ]);
  });

  it("ignores drops that didn't originate from an issue card", () => {
    const rec = makeSummaryRecord("i-aaa", "first", "j-eng", "open");
    cellsByProject = new Map([
      [
        "j-eng",
        new Map<string, BoardCellQuery>([
          ["open", emptyCell({ issues: [rec] })],
          ["in-progress", emptyCell()],
        ]),
      ],
    ]);

    renderBoard();
    const target = screen.getByTestId("board-col-engineering-in-progress");
    const dt = makeDataTransfer();
    // No prior dragStart on a card → dataTransfer has no issue-card payload.
    fireEvent.dragOver(target, { dataTransfer: dt });
    fireEvent.drop(target, { dataTransfer: dt });

    expect(mockUpdateIssue).not.toHaveBeenCalled();
  });
});

describe("IssuesBoard chat button", () => {
  function summaryRecord(issueId: string, projectId: string): IssueSummaryRecord {
    const status: StatusDefinition = makeStatus({ key: "open", label: "Open" });
    const summary: IssueSummary = {
      type: "task",
      title: "Card",
      description: "",
      creator: "alice",
      status,
      project_id: projectId,
      dependencies: [],
      patches: [],
    };
    return {
      issue_id: issueId,
      version: BigInt(1),
      timestamp: "2026-06-09T00:00:00Z",
      issue: summary,
      creation_time: "2026-06-09T00:00:00Z",
    };
  }

  function conversationSummary(
    issueId: string,
    conversationId: string,
    status: ConversationSummary["status"] = "active",
  ): ConversationSummary {
    return {
      conversation_id: conversationId,
      title: null,
      agent_name: null,
      status,
      event_count: 0,
      last_event_preview: null,
      creator: "alice",
      spawned_from: issueId,
      created_at: "2026-06-09T00:00:00Z",
      updated_at: "2026-06-09T00:00:00Z",
    };
  }

  it("renders a chat affordance for issues with a live conversation", () => {
    projectsData = [makeProject("j-eng", "engineering", DEFAULT_STATUSES, "Eng")];
    cellsByProject = new Map([
      [
        "j-eng",
        new Map<string, BoardCellQuery>([
          [
            "open",
            emptyCell({
              issues: [summaryRecord("i-with", "j-eng"), summaryRecord("i-no", "j-eng")],
            }),
          ],
        ]),
      ],
    ]);
    conversationsByIssueMap = new Map([
      ["i-with", conversationSummary("i-with", "c-live")],
    ]);

    renderBoard();

    const link = screen.getByTestId("board-card-conversation-i-with");
    expect(link.getAttribute("href")).toBe("/chat/c-live");
    expect(screen.queryByTestId("board-card-conversation-i-no")).toBeNull();
  });

  it("omits the chat affordance when the conversation map is empty", () => {
    projectsData = [makeProject("j-eng", "engineering", DEFAULT_STATUSES, "Eng")];
    cellsByProject = new Map([
      [
        "j-eng",
        new Map<string, BoardCellQuery>([
          ["open", emptyCell({ issues: [summaryRecord("i-aaa", "j-eng")] })],
        ]),
      ],
    ]);
    conversationsByIssueMap = new Map();

    renderBoard();

    expect(screen.queryByTestId("board-card-conversation-i-aaa")).toBeNull();
  });
});

describe("IssuesBoard mobile single-board view", () => {
  beforeEach(() => {
    mobileMatches = true;
    window.localStorage.clear();
    projectsData = [
      makeProject("j-eng", "engineering", ENG_STATUSES, "Engineering"),
      makeProject("j-design", "design", DEFAULT_STATUSES, "Design"),
    ];
  });

  afterEach(() => {
    window.localStorage.clear();
  });

  it("renders the mobile board picker when multiple projects exist", () => {
    renderBoard();
    expect(screen.getByTestId("board-mobile-picker")).toBeDefined();
  });

  it("shows only the first project section by default", () => {
    renderBoard();
    expect(screen.getByTestId("board-project-engineering")).toBeDefined();
    expect(screen.queryByTestId("board-project-design")).toBeNull();
  });

  it("switching the picker swaps which project section is rendered", () => {
    renderBoard();

    fireEvent.click(
      screen.getByTestId("board-mobile-picker").querySelector("button")!,
    );
    fireEvent.click(screen.getByTestId("board-mobile-picker-option-design"));

    expect(screen.queryByTestId("board-project-engineering")).toBeNull();
    expect(screen.getByTestId("board-project-design")).toBeDefined();
  });

  it("persists the picker selection in localStorage", () => {
    renderBoard();

    fireEvent.click(
      screen.getByTestId("board-mobile-picker").querySelector("button")!,
    );
    fireEvent.click(screen.getByTestId("board-mobile-picker-option-design"));

    expect(
      window.localStorage.getItem("hydra:board-mobile-selected-project"),
    ).toBe("j-design");
  });

  it("rehydrates the picker selection from localStorage on mount", () => {
    window.localStorage.setItem(
      "hydra:board-mobile-selected-project",
      "j-design",
    );
    renderBoard();

    expect(screen.getByTestId("board-project-design")).toBeDefined();
    expect(screen.queryByTestId("board-project-engineering")).toBeNull();
  });

  it("suppresses the picker when scoped to a single project", () => {
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
    });
    render(
      <QueryClientProvider client={client}>
        <MemoryRouter>
          <IssuesBoard
            baseFilters={{ project_id: "j-eng" }}
            filterRootId={null}
          />
        </MemoryRouter>
      </QueryClientProvider>,
    );
    expect(screen.queryByTestId("board-mobile-picker")).toBeNull();
  });

  it("suppresses the picker when only one project exists", () => {
    projectsData = [
      makeProject("j-eng", "engineering", ENG_STATUSES, "Engineering"),
    ];
    renderBoard();
    expect(screen.queryByTestId("board-mobile-picker")).toBeNull();
  });

  it("hides the '+ New project' ghost row when the picker is active", () => {
    renderBoard();
    expect(screen.queryByTestId("board-new-project")).toBeNull();
  });

  it("disables project reordering on mobile (no SortableContext over project ids)", () => {
    renderBoard();
    // With reorder disabled there should only be the per-project status
    // SortableContext, not the project-level one. Status keys are strings.
    for (const items of sortableItemsList) {
      const hasProjectIds = items.some(
        (id) => typeof id === "string" && id.startsWith("j-"),
      );
      expect(hasProjectIds).toBe(false);
    }
  });

  it("falls back to the first project when the stored selection no longer exists", () => {
    window.localStorage.setItem(
      "hydra:board-mobile-selected-project",
      "j-deleted",
    );
    renderBoard();
    expect(screen.getByTestId("board-project-engineering")).toBeDefined();
  });
});
