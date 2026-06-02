import { describe, it, expect, vi, beforeEach } from "vitest";
import { render } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import type { IssueSummaryRecord, Principal } from "@hydra/api";
import { IssueRailRow } from "../RailRow";

function makeRecord(assignee?: Principal | null): IssueSummaryRecord {
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
      assignee: assignee ?? null,
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
  });

  it("does not render an avatar when assignee is absent", () => {
    const { container } = renderRow(makeRecord(null));
    const avatars = container.querySelectorAll('[title^="Assignee"]');
    expect(avatars).toHaveLength(0);
  });

  it("renders an avatar with human kind and the Assignee tooltip for a User principal", () => {
    const principal: Principal = { User: { name: "alice" } };
    const { container } = renderRow(makeRecord(principal));
    const avatar = container.querySelector('[title="Assignee · alice"]');
    expect(avatar).not.toBeNull();
    expect(avatar?.getAttribute("data-kind")).toBe("human");
    expect(avatar?.getAttribute("aria-label")).toBe("Assignee · alice");
  });

  it("renders an avatar with agent kind for an Agent principal", () => {
    const principal: Principal = { Agent: { name: "swe" } };
    const { container } = renderRow(makeRecord(principal));
    const avatar = container.querySelector('[title="Assignee · swe"]');
    expect(avatar).not.toBeNull();
    expect(avatar?.getAttribute("data-kind")).toBe("agent");
  });

  it("places the avatar between the type chip and the AgoTime element in the meta line", () => {
    const principal: Principal = { User: { name: "alice" } };
    const { container } = renderRow(makeRecord(principal));
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
