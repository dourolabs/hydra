// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import React from "react";
import type { IssueVersionRecord, ProjectRecord, ProjectStatusesResponse } from "@hydra/api";
import { makeWrapper } from "./issueDetailHarness";

// --- apiClient mock -----------------------------------------------------

const updateIssueMock = vi.fn<(issueId: string, body: unknown) => Promise<unknown>>();
const listProjectsMock = vi.fn();
const getProjectStatusesMock = vi.fn();
const listConversationsMock = vi.fn();

vi.mock("../../../api/client", () => ({
  apiClient: {
    updateIssue: (issueId: string, body: unknown) => updateIssueMock(issueId, body),
    listProjects: () => listProjectsMock(),
    getProjectStatuses: (projectId: string) => getProjectStatusesMock(projectId),
    listConversations: (query?: unknown) => listConversationsMock(query),
  },
}));

// --- Test-file-only stubs (sibling-component stubs come from the harness) ---

vi.mock("../../toast/useToast", () => ({
  useToast: () => ({ addToast: vi.fn() }),
}));

// --- @hydra/ui stubs (expose Picker rows as buttons) --------------------

vi.mock("@hydra/ui", () => ({
  Avatar: () => null,
  Badge: () => null,
  TypeChip: () => null,
  Button: ({ children, onClick }: { children: React.ReactNode; onClick?: () => void }) => (
    <button onClick={onClick}>{children}</button>
  ),
  Icons: new Proxy(
    {},
    {
      get: () => () => <span aria-hidden="true" />,
    },
  ),
  Picker: ({
    label,
    value,
    open,
    onToggle,
    children,
    "data-testid": testId,
  }: {
    label: string;
    value: React.ReactNode;
    open: boolean;
    onToggle: () => void;
    children: React.ReactNode;
    "data-testid"?: string;
  }) => (
    <div data-testid={testId ?? "picker"}>
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={open}
        aria-label={label}
        data-testid={testId ? `${testId}-trigger` : undefined}
      >
        {value}
      </button>
      {open && <div data-testid={testId ? `${testId}-pop` : "picker-pop"}>{children}</div>}
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
      onClick={onClick}
      data-active={active ? "true" : "false"}
      data-testid={testId}
    >
      {children}
    </button>
  ),
}));

vi.mock("../IssueProjectPicker.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));
vi.mock("../IssueStatusPicker.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));
vi.mock("../../projects/StatusChip.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const { IssueDetail } = await import("../IssueDetail");

// --- Fixtures -----------------------------------------------------------

const projectAlpha: ProjectRecord = {
  project_id: "j-alpha",
  version: 1,
  project: {
    key: "alpha",
    name: "Alpha",
    statuses: [],
    archived: false,
  } as unknown as ProjectRecord["project"],
};

const projectBeta: ProjectRecord = {
  project_id: "j-beta",
  version: 1,
  project: {
    key: "beta",
    name: "Beta",
    statuses: [],
    archived: false,
  } as unknown as ProjectRecord["project"],
};

const alphaStatuses: ProjectStatusesResponse = {
  statuses: [
    {
      key: "in-progress",
      label: "In Progress",
      color: "#f1c40f",
      unblocks_parents: false,
      unblocks_dependents: false,
      cascades_to_children: false,
      position: 0,
    },
    {
      key: "review",
      label: "Review",
      color: "#3498db",
      unblocks_parents: false,
      unblocks_dependents: false,
      cascades_to_children: false,
      position: 1,
    },
  ] as unknown as ProjectStatusesResponse["statuses"],
};

const betaStatuses: ProjectStatusesResponse = {
  statuses: [
    {
      key: "triage",
      label: "Triage",
      color: "#9b59b6",
      unblocks_parents: false,
      unblocks_dependents: false,
      cascades_to_children: false,
      position: 0,
    },
    {
      key: "shipping",
      label: "Shipping",
      color: "#2ecc71",
      unblocks_parents: false,
      unblocks_dependents: false,
      cascades_to_children: false,
      position: 1,
    },
  ] as unknown as ProjectStatusesResponse["statuses"],
};

function makeRecord(): IssueVersionRecord {
  return {
    issue_id: "i-1",
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    labels: [],
    issue: {
      type: "task",
      title: "Sample",
      description: "",
      creator: "alice",
      progress: "",
      status: {
        key: "in-progress",
        label: "In Progress",
        color: "#f1c40f",
        unblocks_parents: false,
        unblocks_dependents: false,
        cascades_to_children: false,
        position: 0,
      },
      project_id: "j-alpha",
      assignee: null,
      dependencies: [],
      patches: [],
      labels: [],
      archived: false,
    },
  } as unknown as IssueVersionRecord;
}

beforeEach(() => {
  updateIssueMock.mockReset();
  updateIssueMock.mockResolvedValue({});
  listProjectsMock.mockReset();
  listProjectsMock.mockResolvedValue({ projects: [projectAlpha, projectBeta] });
  getProjectStatusesMock.mockReset();
  getProjectStatusesMock.mockImplementation(async (projectId: string) =>
    projectId === "j-beta" ? betaStatuses : alphaStatuses,
  );
  listConversationsMock.mockReset();
  listConversationsMock.mockResolvedValue({ conversations: [] });
});

async function openProjectPicker() {
  await waitFor(() => expect(screen.getByTestId("issue-project-option-alpha")).toBeDefined());
}

async function openStatusPicker() {
  await waitFor(() => expect(screen.getByTestId("issue-status-option-in-progress")).toBeDefined());
}

describe("IssueDetail project + status coordination", () => {
  it("does NOT fire updateIssue when the user picks a different project", async () => {
    render(<IssueDetail record={makeRecord()} />, { wrapper: makeWrapper() });

    // Open project picker.
    fireEvent.click(screen.getByTestId("issue-project-picker-trigger"));
    await openProjectPicker();

    // Pick the OTHER project.
    fireEvent.click(screen.getByTestId("issue-project-option-beta"));

    // No mutation should fire.
    expect(updateIssueMock).not.toHaveBeenCalled();

    // The project pill should show beta now.
    expect(screen.getByTestId("issue-project-picker-trigger").textContent).toContain("beta");

    // The status picker should show the "Select a status…" placeholder.
    expect(screen.getByTestId("issue-status-picker-trigger").textContent).toContain(
      "Select a status…",
    );
  });

  it("shows the pending project's statuses (not the persisted project's)", async () => {
    render(<IssueDetail record={makeRecord()} />, { wrapper: makeWrapper() });

    // Set pending project to beta.
    fireEvent.click(screen.getByTestId("issue-project-picker-trigger"));
    await openProjectPicker();
    fireEvent.click(screen.getByTestId("issue-project-option-beta"));

    // Open the status picker; it should fetch / list beta's statuses.
    fireEvent.click(screen.getByTestId("issue-status-picker-trigger"));
    await waitFor(() => expect(screen.getByTestId("issue-status-option-triage")).toBeDefined());

    expect(screen.getByTestId("issue-status-option-shipping")).toBeDefined();
    // Persisted-project statuses must NOT appear.
    expect(screen.queryByTestId("issue-status-option-in-progress")).toBeNull();
    expect(screen.queryByTestId("issue-status-option-review")).toBeNull();
  });

  it("fires ONE atomic updateIssue with both project_id and status when the user picks a status in pending mode", async () => {
    render(<IssueDetail record={makeRecord()} />, { wrapper: makeWrapper() });

    fireEvent.click(screen.getByTestId("issue-project-picker-trigger"));
    await openProjectPicker();
    fireEvent.click(screen.getByTestId("issue-project-option-beta"));

    fireEvent.click(screen.getByTestId("issue-status-picker-trigger"));
    await waitFor(() => expect(screen.getByTestId("issue-status-option-shipping")).toBeDefined());

    await act(async () => {
      fireEvent.click(screen.getByTestId("issue-status-option-shipping"));
    });

    expect(updateIssueMock).toHaveBeenCalledTimes(1);
    const [issueId, body] = updateIssueMock.mock.calls[0] as [
      string,
      { issue: { status: string; project_id: string }; session_id: null },
    ];
    expect(issueId).toBe("i-1");
    expect(body.session_id).toBeNull();
    expect(body.issue.status).toBe("shipping");
    expect(body.issue.project_id).toBe("j-beta");

    // After success, the status pill is no longer in placeholder mode.
    await waitFor(() => {
      expect(screen.getByTestId("issue-status-picker-trigger").textContent).not.toContain(
        "Select a status…",
      );
    });
  });

  it("clears pending state and reverts the status placeholder when the user re-picks the original project", async () => {
    render(<IssueDetail record={makeRecord()} />, { wrapper: makeWrapper() });

    // Enter pending: alpha → beta.
    fireEvent.click(screen.getByTestId("issue-project-picker-trigger"));
    await openProjectPicker();
    fireEvent.click(screen.getByTestId("issue-project-option-beta"));

    expect(screen.getByTestId("issue-status-picker-trigger").textContent).toContain(
      "Select a status…",
    );

    // Re-pick alpha (the persisted project).
    fireEvent.click(screen.getByTestId("issue-project-picker-trigger"));
    await openProjectPicker();
    fireEvent.click(screen.getByTestId("issue-project-option-alpha"));

    // No mutation fired during the cancel path.
    expect(updateIssueMock).not.toHaveBeenCalled();

    // Project pill back to alpha.
    expect(screen.getByTestId("issue-project-picker-trigger").textContent).toContain("alpha");
    // Status picker no longer shows the placeholder.
    expect(screen.getByTestId("issue-status-picker-trigger").textContent).not.toContain(
      "Select a status…",
    );
    expect(screen.getByTestId("issue-status-picker-trigger").textContent).toContain("In Progress");
  });

  it("auto-commits status when NO pending project change is active (legacy path preserved)", async () => {
    render(<IssueDetail record={makeRecord()} />, { wrapper: makeWrapper() });

    fireEvent.click(screen.getByTestId("issue-status-picker-trigger"));
    await openStatusPicker();

    await act(async () => {
      fireEvent.click(screen.getByTestId("issue-status-option-review"));
    });

    expect(updateIssueMock).toHaveBeenCalledTimes(1);
    const [, body] = updateIssueMock.mock.calls[0] as [
      string,
      { issue: { status: string; project_id: string } },
    ];
    expect(body.issue.status).toBe("review");
    expect(body.issue.project_id).toBe("j-alpha");
  });
});
