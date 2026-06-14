import { describe, it, expect, vi, beforeEach } from "vitest";
import { render } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import type { IssueVersionRecord, ProjectRecord } from "@hydra/api";

const useIssueMock = vi.fn<(id: string) => {
  data: IssueVersionRecord | undefined;
  isLoading: boolean;
  isError: boolean;
}>();
const useProjectsMock = vi.fn<() => { data: ProjectRecord[] | undefined }>();

vi.mock("../../../issues/useIssue", () => ({
  useIssue: (id: string) => useIssueMock(id),
}));
vi.mock("../../../projects/useProjects", () => ({
  useProjects: () => useProjectsMock(),
}));

const { IssuePreviewCard } = await import("../IssuePreviewCard");

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

function makeIssueRecord(opts?: {
  projectId?: string | null;
  statusLabel?: string;
}): IssueVersionRecord {
  return {
    issue_id: "i-42",
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    issue: {
      type: "task",
      title: "Add login button",
      description: "Add a login button to the homepage",
      creator: "alice",
      status: {
        key: "open",
        label: opts?.statusLabel ?? "Open",
        color: "#3498db",
        position: 0,
        unblocks_parents: false,
        unblocks_dependents: false,
        cascades_to_children: false,
      },
      project_id: opts?.projectId ?? "j-defaul",
      assignee: null,
      dependencies: [],
      patches: [],
    },
  };
}

function renderCard(id: string) {
  return render(
    <MemoryRouter>
      <IssuePreviewCard id={id} />
    </MemoryRouter>,
  );
}

describe("IssuePreviewCard", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useProjectsMock.mockReturnValue({ data: SEEDED_PROJECTS });
  });

  it("renders a ProjectChip in the topRow for an issue under the default project", () => {
    useIssueMock.mockReturnValue({
      data: makeIssueRecord({ projectId: null }),
      isLoading: false,
      isError: false,
    });
    const { getByTestId } = renderCard("i-42");
    const chip = getByTestId("issue-preview-project-chip-i-42");
    expect(chip.textContent).toBe("default");
  });

  it("renders a ProjectChip with the matched project key when project_id is set", () => {
    useIssueMock.mockReturnValue({
      data: makeIssueRecord({ projectId: "j-engv2" }),
      isLoading: false,
      isError: false,
    });
    const { getByTestId } = renderCard("i-42");
    const chip = getByTestId("issue-preview-project-chip-i-42");
    expect(chip.textContent).toBe("engineering-v2");
  });

  it("places the ProjectChip between StatusChip and MonoId in topRow", () => {
    useIssueMock.mockReturnValue({
      data: makeIssueRecord({ projectId: "j-engv2" }),
      isLoading: false,
      isError: false,
    });
    const { container, getByTestId } = renderCard("i-42");
    const chip = getByTestId("issue-preview-project-chip-i-42");
    const topRow = chip.parentElement!;
    const children = Array.from(topRow.children);
    const chipIdx = children.indexOf(chip);
    const monoIdEl = children.find((el) => el.getAttribute("data-pc-mono") === "true");
    expect(monoIdEl).toBeDefined();
    const monoIdx = children.indexOf(monoIdEl!);
    expect(chipIdx).toBeGreaterThan(0);
    expect(monoIdx).toBeGreaterThan(chipIdx);
    // Ensure the topRow renders inside the card (not a stray div elsewhere).
    expect(container.querySelector("button")).not.toBeNull();
  });

  it("does not render a ProjectChip when projects are not yet loaded", () => {
    useProjectsMock.mockReturnValue({ data: undefined });
    useIssueMock.mockReturnValue({
      data: makeIssueRecord({ projectId: null }),
      isLoading: false,
      isError: false,
    });
    const { queryByTestId } = renderCard("i-42");
    expect(queryByTestId("issue-preview-project-chip-i-42")).toBeNull();
  });

  it("renders the SkeletonPreviewCard while loading", () => {
    useIssueMock.mockReturnValue({
      data: undefined,
      isLoading: true,
      isError: false,
    });
    const { container } = renderCard("i-42");
    expect(container.querySelector('[aria-label^="Loading"]')).not.toBeNull();
  });
});
