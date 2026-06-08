// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import type {
  IssueSummaryRecord,
  ProjectRecord,
  StatusDefinition,
} from "@hydra/api";

let projectsData: ProjectRecord[] | undefined = [];
vi.mock("../../../projects/useProjects", () => ({
  useProjects: () => ({ data: projectsData }),
}));

vi.mock("@hydra/ui", () => ({
  Avatar: ({ name }: { name: string }) => (
    <span data-testid="avatar">{name}</span>
  ),
  Icons: new Proxy(
    {},
    {
      get: () => () => <span data-testid="icon" />,
    },
  ),
}));

vi.mock("../IssuesCards.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("../../../projects/StatusChip.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("../../../projects/ProjectChip.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const { IssuesCards } = await import("../IssuesCards");
const { IssuesView } = await import("../IssuesView");

// IssuesView pulls in IssuesBoard, which needs these.
vi.mock("../IssuesBoard", () => ({
  IssuesBoard: () => <div data-testid="issues-board" />,
}));

vi.mock("../IssuesTable", () => ({
  IssuesTable: () => <div data-testid="issues-table" />,
}));

vi.mock("../../../filters", () => ({
  FilterBar: () => <div data-testid="filter-bar" />,
}));

vi.mock("../IssuesView.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const STATUSES_DEFAULT: StatusDefinition[] = [
  {
    key: "open",
    label: "Open",
    color: "#3498db",
    unblocks_parents: false,
    unblocks_dependents: false,
    cascades_to_children: false,
  },
  {
    key: "in-progress",
    label: "In progress",
    color: "#f1c40f",
    unblocks_parents: false,
    unblocks_dependents: false,
    cascades_to_children: false,
  },
];

const STATUSES_ALPHA: StatusDefinition[] = [
  {
    key: "backlog",
    label: "Backlog",
    color: "#9b59b6",
    unblocks_parents: false,
    unblocks_dependents: false,
    cascades_to_children: false,
  },
];

function makeProject(
  id: string,
  key: string,
  name: string,
  statuses: StatusDefinition[] = STATUSES_DEFAULT,
): ProjectRecord {
  return {
    project_id: id,
    version: 1,
    project: {
      key,
      name,
      statuses,
      default_status_key: statuses[0].key,
      creator: "alice",
      deleted: false,
    },
  };
}

function makeIssue(
  id: string,
  overrides: Partial<IssueSummaryRecord["issue"]> = {},
  resolvedOverride: StatusDefinition | null | undefined = undefined,
): IssueSummaryRecord {
  const status = overrides.status ?? "open";
  const resolved_status: StatusDefinition | null =
    resolvedOverride !== undefined
      ? resolvedOverride
      : {
          key: status,
          label: status,
          color: "#3498db",
          unblocks_parents: false,
          unblocks_dependents: false,
          cascades_to_children: false,
        };
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
      status,
      project_id: null,
      assignee: null,
      progress: "",
      dependencies: [],
      patches: [],
      labels: [],
      resolved_status,
      ...overrides,
    },
  } as unknown as IssueSummaryRecord;
}

function renderCards(props: {
  issues: IssueSummaryRecord[];
  filterRootId?: string | null;
}) {
  return render(
    <MemoryRouter>
      <IssuesCards
        issues={props.issues}
        childStatusMap={new Map()}
        sessionsByIssue={new Map()}
        filterRootId={props.filterRootId ?? null}
      />
    </MemoryRouter>,
  );
}

beforeEach(() => {
  projectsData = [
    makeProject("j-defaul", "default", "Default"),
    makeProject("j-altpro", "alpha", "Alpha", STATUSES_ALPHA),
  ];
});

afterEach(() => {
  cleanup();
});

describe("IssuesCards grouping", () => {
  it("groups issues under per-project section headers", () => {
    renderCards({
      issues: [
        makeIssue("i-a", { project_id: "j-defaul", status: "open" }),
        makeIssue("i-b", { project_id: "j-defaul", status: "in-progress" }),
        makeIssue("i-c", { project_id: "j-altpro", status: "backlog" }),
      ],
    });

    expect(screen.getByTestId("issues-cards-group-default")).toBeDefined();
    expect(screen.getByTestId("issues-cards-group-alpha")).toBeDefined();
    expect(screen.getByTestId("issues-card-i-a")).toBeDefined();
    expect(screen.getByTestId("issues-card-i-b")).toBeDefined();
    expect(screen.getByTestId("issues-card-i-c")).toBeDefined();
  });

  it("places issues with null project_id under the synthesized default-project section", () => {
    renderCards({
      issues: [makeIssue("i-null", { project_id: null, status: "open" })],
    });

    const defaultGroup = screen.getByTestId("issues-cards-group-default");
    expect(defaultGroup).toBeDefined();
    expect(
      within(defaultGroup).getByTestId("issues-card-i-null"),
    ).toBeDefined();
    // The Default chip is labelled with its slug `default`.
    expect(
      within(defaultGroup).getByTestId("issues-cards-group-chip-default"),
    ).toBeDefined();
  });

  it("renders a flat ungrouped grid when projects haven't loaded", () => {
    projectsData = undefined;
    renderCards({
      issues: [
        makeIssue("i-a", { project_id: "j-defaul" }),
        makeIssue("i-b", { project_id: "j-altpro" }),
      ],
    });

    expect(screen.queryByTestId("issues-cards-group-default")).toBeNull();
    expect(screen.getByTestId("issues-card-i-a")).toBeDefined();
    expect(screen.getByTestId("issues-card-i-b")).toBeDefined();
  });

  it("counts issues per section", () => {
    renderCards({
      issues: [
        makeIssue("i-a", { project_id: "j-defaul" }),
        makeIssue("i-b", { project_id: "j-defaul" }),
        makeIssue("i-c", { project_id: "j-altpro" }),
      ],
    });

    const defaultGroup = screen.getByTestId("issues-cards-group-default");
    const alphaGroup = screen.getByTestId("issues-cards-group-alpha");
    expect(within(defaultGroup).getByText("2 issues")).toBeDefined();
    expect(within(alphaGroup).getByText("1 issue")).toBeDefined();
  });
});

describe("IssuesCards status chip", () => {
  it("renders the project-resolved status definition (dot + label) per card", () => {
    renderCards({
      issues: [
        makeIssue(
          "i-a",
          { project_id: "j-altpro", status: "backlog" },
          {
            key: "backlog",
            label: "Backlog",
            color: "#9b59b6",
            unblocks_parents: false,
            unblocks_dependents: false,
            cascades_to_children: false,
          },
        ),
      ],
    });

    const card = screen.getByTestId("issues-card-i-a");
    expect(within(card).getByText("Backlog")).toBeDefined();
  });

  it("falls back to the bare status key when resolved_status is null", () => {
    renderCards({
      issues: [
        makeIssue(
          "i-a",
          { project_id: "j-defaul", status: "custom-state" },
          null,
        ),
      ],
    });

    const card = screen.getByTestId("issues-card-i-a");
    expect(within(card).getByText("custom-state")).toBeDefined();
  });
});

describe("IssuesCards BLOCKED tag", () => {
  it("renders BLOCKED on a card with an open blocked-on dep", () => {
    const blocker = makeIssue("i-blocker", {
      project_id: "j-defaul",
      status: "open",
    });
    const blocked = makeIssue("i-blocked", {
      project_id: "j-defaul",
      status: "open",
      dependencies: [{ type: "blocked-on", issue_id: "i-blocker" }],
    });

    renderCards({ issues: [blocker, blocked] });

    expect(screen.getByTestId("issues-card-blocked-i-blocked")).toBeDefined();
    expect(screen.queryByTestId("issues-card-blocked-i-blocker")).toBeNull();
  });

  it("does NOT render BLOCKED when the blocked-on target is closed", () => {
    const closer = makeIssue("i-closer", {
      project_id: "j-defaul",
      status: "closed",
    });
    const candidate = makeIssue("i-cand", {
      project_id: "j-defaul",
      status: "open",
      dependencies: [{ type: "blocked-on", issue_id: "i-closer" }],
    });

    renderCards({ issues: [closer, candidate] });

    expect(screen.queryByTestId("issues-card-blocked-i-cand")).toBeNull();
  });
});

describe("IssuesCards assignee", () => {
  it("renders the assignee avatar + name when present", () => {
    renderCards({
      issues: [
        makeIssue("i-a", {
          project_id: "j-defaul",
          assignee: { User: { name: "alice" } },
        }),
      ],
    });

    const card = screen.getByTestId("issues-card-i-a");
    const avatar = within(card).getByTestId("avatar");
    expect(avatar.textContent).toBe("alice");
    // Name appears twice (avatar + name span); the displayed name span sits
    // alongside the avatar in the card footer.
    expect(within(card).getAllByText("alice").length).toBeGreaterThan(0);
  });
});

describe("IssuesView Cards segment", () => {
  it("renders three segmented toggle buttons including Cards", () => {
    render(
      <MemoryRouter>
        <IssuesView
          layout="table"
          onLayoutChange={() => {}}
          issues={[]}
          childStatusMap={new Map()}
          sessionsByIssue={new Map()}
          isLoading={false}
          hasNextPage={false}
          isFetchingNextPage={false}
          onLoadMore={() => {}}
          baseFilters={{}}
          username="alice"
          filterRootId={null}
          eyebrow="ALL · 0 ISSUES"
          title="All issues"
          filters={[]}
          setFilters={() => {}}
          definitions={{} as never}
          filteredCount={0}
          totalCount={0}
          searchValue=""
          onSearchChange={() => {}}
        />
      </MemoryRouter>,
    );

    expect(screen.getByTestId("issues-layout-table")).toBeDefined();
    expect(screen.getByTestId("issues-layout-board")).toBeDefined();
    expect(screen.getByTestId("issues-layout-cards")).toBeDefined();
  });

  it("invokes onLayoutChange('cards') when the Cards toggle is clicked", () => {
    const onLayoutChange = vi.fn();
    render(
      <MemoryRouter>
        <IssuesView
          layout="table"
          onLayoutChange={onLayoutChange}
          issues={[]}
          childStatusMap={new Map()}
          sessionsByIssue={new Map()}
          isLoading={false}
          hasNextPage={false}
          isFetchingNextPage={false}
          onLoadMore={() => {}}
          baseFilters={{}}
          username="alice"
          filterRootId={null}
          eyebrow=""
          title=""
          filters={[]}
          setFilters={() => {}}
          definitions={{} as never}
          filteredCount={0}
          totalCount={0}
          searchValue=""
          onSearchChange={() => {}}
        />
      </MemoryRouter>,
    );

    fireEvent.click(screen.getByTestId("issues-layout-cards"));
    expect(onLayoutChange).toHaveBeenCalledWith("cards");
  });

  it("routes body rendering to IssuesCards when layout='cards' and there are issues", () => {
    render(
      <MemoryRouter>
        <IssuesView
          layout="cards"
          onLayoutChange={() => {}}
          issues={[makeIssue("i-a", { project_id: "j-defaul" })]}
          childStatusMap={new Map()}
          sessionsByIssue={new Map()}
          isLoading={false}
          hasNextPage={false}
          isFetchingNextPage={false}
          onLoadMore={() => {}}
          baseFilters={{}}
          username="alice"
          filterRootId={null}
          eyebrow=""
          title=""
          filters={[]}
          setFilters={() => {}}
          definitions={{} as never}
          filteredCount={1}
          totalCount={1}
          searchValue=""
          onSearchChange={() => {}}
        />
      </MemoryRouter>,
    );

    expect(screen.getByTestId("issues-card-i-a")).toBeDefined();
    expect(screen.queryByTestId("issues-table")).toBeNull();
    expect(screen.queryByTestId("issues-board")).toBeNull();
  });
});
