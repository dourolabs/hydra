import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import React from "react";
import type { IssueVersionRecord } from "@hydra/api";

const useIssuesByIdsMock = vi.fn<
  (ids: string[]) => Map<string, IssueVersionRecord>
>(() => new Map());

vi.mock("../useIssue", () => ({
  useIssue: () => ({ data: undefined }),
  useIssuesByIds: (ids: string[]) => useIssuesByIdsMock(ids),
}));

const useProjectMock = vi.fn();
const useProjectsMock = vi.fn();
vi.mock("../../projects/useProjects", () => ({
  useProject: (id: string | null) => useProjectMock(id),
  useProjects: () => useProjectsMock(),
}));

vi.mock("../IssueLabelEditor", () => ({
  IssueLabelEditor: () => <div data-testid="label-editor" />,
}));

vi.mock("../IssueSettingsEditor", () => ({
  IssueSettingsEditor: () => <div data-testid="issue-settings-display" />,
}));

vi.mock("@hydra/ui", () => ({
  Avatar: ({ name, kind }: { name: string; kind?: string }) => (
    <span data-testid="avatar" data-kind={kind ?? "human"}>
      {name}
    </span>
  ),
  Badge: ({ status }: { status: string }) => (
    <span data-testid={`badge-${status}`}>{status}</span>
  ),
  TypeChip: ({ type }: { type: string }) => <span data-testid={`type-${type}`}>{type}</span>,
}));

vi.mock("../../projects/ProjectChip", () => ({
  ProjectChip: ({ projectKey, name }: { projectKey: string; name?: string | null }) => (
    <span data-testid="project-chip" data-key={projectKey}>
      {name ?? projectKey}
    </span>
  ),
}));

vi.mock("../../projects/StatusChip", () => ({
  StatusChip: () => <span data-testid="status-chip-inner" />,
}));

vi.mock("react-router-dom", () => ({
  Link: ({ children, to }: { children: React.ReactNode; to: string }) => (
    <a href={to}>{children}</a>
  ),
}));

vi.mock("../IssueDetailsTab.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const { IssueDetailsTab } = await import("../IssueDetailsTab");

function makeRecord(
  overrides: Partial<IssueVersionRecord["issue"]> = {},
  recordOverrides: Partial<IssueVersionRecord> = {},
): IssueVersionRecord {
  return {
    issue_id: "i-1",
    version: 1n,
    timestamp: "2026-01-02T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    labels: [],
    ...recordOverrides,
    issue: {
      type: "task",
      title: "Sample",
      description: "",
      creator: "alice",
      status: "open",
      progress: "",
      dependencies: [],
      patches: [],
      labels: [],
      ...overrides,
    },
  } as unknown as IssueVersionRecord;
}

function makeDepRecord(
  id: string,
  status: string,
): IssueVersionRecord {
  return {
    issue_id: id,
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    issue: {
      type: "task",
      title: `Title ${id}`,
      description: "",
      creator: "alice",
      status: {
        key: status,
        label: status,
        color: "#888888",
        unblocks_parents: false,
        unblocks_dependents: false,
        cascades_to_children: false,
      },
      progress: "",
      dependencies: [],
      patches: [],
    },
  } as unknown as IssueVersionRecord;
}

describe("IssueDetailsTab", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useIssuesByIdsMock.mockReturnValue(new Map());
    useProjectMock.mockReturnValue({ data: undefined });
    useProjectsMock.mockReturnValue({ data: undefined });
  });

  it("renders status chip, assignee, type, created, updated, and labels editor", () => {
    render(
      <IssueDetailsTab
        record={makeRecord({
          assignee: { User: { name: "bob" } },
          type: "feature",
        })}
        onOpenStatusModal={() => {}}
      />,
    );
    expect(screen.getByTestId("status-chip")).toBeDefined();
    const avatar = screen.getByTestId("avatar");
    expect(avatar.getAttribute("data-kind")).toBe("human");
    expect(avatar.textContent).toBe("bob");
    expect(screen.getByText("Created")).toBeDefined();
    expect(screen.getByText("Updated")).toBeDefined();
    expect(screen.getByTestId("label-editor")).toBeDefined();
  });

  it("renders agent-kind avatar when assignee is an Agent principal", () => {
    render(
      <IssueDetailsTab
        record={makeRecord({
          assignee: { Agent: { name: "swe" } },
        })}
        onOpenStatusModal={() => {}}
      />,
    );
    const avatar = screen.getByTestId("avatar");
    expect(avatar.getAttribute("data-kind")).toBe("agent");
    expect(avatar.textContent).toBe("swe");
  });

  it("does not render the Parent block (parents live in the Related tab)", () => {
    render(
      <IssueDetailsTab
        record={makeRecord({
          dependencies: [{ type: "child-of", issue_id: "i-parent" }],
        })}
        onOpenStatusModal={() => {}}
      />,
    );
    expect(screen.queryByText("Parent")).toBeNull();
  });

  it("renders the Session settings block with an editor", () => {
    render(
      <IssueDetailsTab
        record={makeRecord({
          session_settings: { repo_name: "dourolabs/hydra", branch: "main" },
        })}
        onOpenStatusModal={() => {}}
      />,
    );
    expect(screen.getByText("Session settings")).toBeDefined();
    expect(screen.getByTestId("issue-settings-display")).toBeDefined();
  });

  it("renders the Session settings editor even when no settings are configured", () => {
    render(
      <IssueDetailsTab record={makeRecord({})} onOpenStatusModal={() => {}} />,
    );
    expect(screen.getByText("Session settings")).toBeDefined();
    expect(screen.getByTestId("issue-settings-display")).toBeDefined();
  });

  it("shows 'Unassigned' italic when assignee is missing", () => {
    render(
      <IssueDetailsTab record={makeRecord({})} onOpenStatusModal={() => {}} />,
    );
    expect(screen.getByText("Unassigned")).toBeDefined();
  });

  it("renders Blocked on block with dep rows when blocked-on deps exist", () => {
    render(
      <IssueDetailsTab
        record={makeRecord({
          dependencies: [{ type: "blocked-on", issue_id: "i-blocker" }],
        })}
        onOpenStatusModal={() => {}}
      />,
    );
    expect(screen.getByText("Blocked on")).toBeDefined();
    expect(screen.getByText("i-blocker")).toBeDefined();
  });

  it("invokes onOpenStatusModal when the status chip is clicked", () => {
    const onOpen = vi.fn();
    render(<IssueDetailsTab record={makeRecord({})} onOpenStatusModal={onOpen} />);
    fireEvent.click(screen.getByTestId("status-chip"));
    expect(onOpen).toHaveBeenCalled();
  });

  describe("Project block", () => {
    it("renders the resolved project chip when issue has a project_id", () => {
      useProjectMock.mockReturnValue({
        data: {
          project_id: "j-engv2",
          version: 1,
          project: { key: "engineering-v2", name: "Engineering v2" },
        },
      });
      render(
        <IssueDetailsTab
          record={makeRecord({ project_id: "j-engv2" })}
          onOpenStatusModal={() => {}}
        />,
      );
      expect(useProjectMock).toHaveBeenCalledWith("j-engv2");
      const chip = screen.getByTestId("project-chip");
      expect(chip.getAttribute("data-key")).toBe("engineering-v2");
      expect(chip.textContent).toBe("Engineering v2");
    });

    it("renders the seeded default project chip when issue.project_id is j-defaul", () => {
      useProjectMock.mockReturnValue({
        data: {
          project_id: "j-defaul",
          version: 1,
          project: { key: "default", name: "Default" },
        },
      });
      render(
        <IssueDetailsTab
          record={makeRecord({ project_id: "j-defaul" })}
          onOpenStatusModal={() => {}}
        />,
      );
      expect(useProjectMock).toHaveBeenCalledWith("j-defaul");
      const chip = screen.getByTestId("project-chip");
      expect(chip.getAttribute("data-key")).toBe("default");
      expect(chip.textContent).toBe("Default");
    });
  });

  it("never renders a BLOCKED tag on the Details tab, even when a blocked-on dep is open", () => {
    useIssuesByIdsMock.mockReturnValue(
      new Map([["i-blocker", makeDepRecord("i-blocker", "open")]]),
    );
    render(
      <IssueDetailsTab
        record={makeRecord({
          dependencies: [{ type: "blocked-on", issue_id: "i-blocker" }],
        })}
        onOpenStatusModal={() => {}}
      />,
    );
    expect(screen.queryByTestId("blocked-tag")).toBeNull();
  });
});
