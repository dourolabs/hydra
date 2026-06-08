// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { ProjectRecord } from "@hydra/api";

vi.mock("@hydra/ui", () => ({
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
}));

vi.mock("../ProjectEditor", () => ({
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

vi.mock("../../../components/LargeModal.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const { ProjectSettingsModal } = await import("../ProjectSettingsModal");

function makeProject(): ProjectRecord {
  return {
    project_id: "j-engine",
    version: 1,
    project: {
      key: "engineering",
      name: "Engineering",
      statuses: [
        {
          key: "open",
          label: "Open",
          color: "#3498db",
          unblocks_parents: false,
          unblocks_dependents: false,
          cascades_to_children: false,
        },
      ],
      default_status_key: "open",
      creator: "alice",
      deleted: false,
    },
  };
}

afterEach(() => {
  cleanup();
});

describe("ProjectSettingsModal", () => {
  it("renders nothing when closed", () => {
    render(
      <ProjectSettingsModal
        open={false}
        onClose={() => {}}
        project={makeProject()}
      />,
    );

    expect(screen.queryByTestId("modal")).toBeNull();
    expect(screen.queryByTestId("project-editor")).toBeNull();
  });

  it("wraps ProjectEditor in a modal seeded with the project record", () => {
    render(
      <ProjectSettingsModal
        open
        onClose={() => {}}
        project={makeProject()}
      />,
    );

    expect(screen.getByTestId("modal")).toBeDefined();
    expect(screen.getByTestId("modal-title").textContent).toContain(
      "Engineering",
    );

    const editor = screen.getByTestId("project-editor");
    expect(editor.getAttribute("data-project-id")).toBe("j-engine");
    expect(editor.getAttribute("data-project-key")).toBe("engineering");
    expect(editor.getAttribute("data-project-name")).toBe("Engineering");
    expect(editor.getAttribute("data-creator")).toBe("alice");
  });

  it("invokes onClose when the close button is clicked", () => {
    const onClose = vi.fn();
    render(
      <ProjectSettingsModal
        open
        onClose={onClose}
        project={makeProject()}
      />,
    );

    fireEvent.click(screen.getByTestId("modal-close"));
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
