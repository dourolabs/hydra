// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, cleanup, within } from "@testing-library/react";
import React from "react";
import type { SessionSummaryRecord } from "@hydra/api";

const sessionsState: { data: SessionSummaryRecord[] | undefined } = {
  data: undefined,
};

vi.mock("../useSessionsByIssue", () => ({
  useSessionsByIssue: () => ({
    data: sessionsState.data,
    isLoading: false,
    error: null,
    refetch: vi.fn(),
  }),
}));

vi.mock("react-router-dom", () => ({
  Link: ({
    to,
    children,
    className,
  }: {
    to: string;
    children: React.ReactNode;
    className?: string;
  }) => (
    <a href={to} className={className}>
      {children}
    </a>
  ),
}));

vi.mock("@hydra/ui", () => ({
  Badge: ({ status }: { status: string }) => (
    <span data-testid="badge">{status}</span>
  ),
  Spinner: () => <span data-testid="spinner" />,
}));

vi.mock("../../../utils/badgeStatus", () => ({
  normalizeSessionStatus: (s: string) => s,
}));

vi.mock("../../../utils/time", () => ({
  getRuntime: () => "1m",
}));

vi.mock("../SessionList.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("../TokensCell.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("../../../components/LoadingState/LoadingState.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("../../../components/ErrorState/ErrorState.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const { SessionList } = await import("../SessionList");

function rec(
  id: string,
  overrides: Partial<SessionSummaryRecord["session"]> = {},
): SessionSummaryRecord {
  return {
    session_id: id,
    version: 1n,
    timestamp: "2026-03-15T10:00:00.000Z",
    session: {
      prompt: "do the thing",
      creator: "swe",
      status: "complete",
      start_time: "2026-03-15T10:00:00.000Z",
      end_time: "2026-03-15T10:01:00.000Z",
      ...overrides,
    },
  } as SessionSummaryRecord;
}

describe("SessionList Tokens column", () => {
  beforeEach(() => {
    sessionsState.data = undefined;
    cleanup();
  });

  it("renders a Tokens header between Runtime and Logs", () => {
    sessionsState.data = [rec("t-1")];
    render(<SessionList issueId="i-1" />);

    const headers = screen
      .getAllByRole("columnheader")
      .map((th) => th.textContent ?? "");
    const runtimeIdx = headers.indexOf("Runtime");
    const tokensIdx = headers.indexOf("Tokens");
    const logsIdx = headers.indexOf("Logs");

    expect(runtimeIdx).toBeGreaterThan(-1);
    expect(tokensIdx).toBe(runtimeIdx + 1);
    expect(logsIdx).toBe(tokensIdx + 1);
  });

  it("shows summed input + cache tokens via the shared cell (matches /sessions formatting)", () => {
    const r = rec("t-1", {
      usage: {
        input_tokens: 1000n,
        cache_read_input_tokens: 2000n,
        cache_creation_input_tokens: 500n,
        output_tokens: 750n,
      },
    });
    sessionsState.data = [r];

    render(<SessionList issueId="i-1" />);

    const list = screen.getByTestId("session-list");
    const tokens = list.querySelector("[title]") as HTMLElement | null;
    expect(tokens).not.toBeNull();
    const title = tokens!.getAttribute("title") ?? "";
    expect(title).toContain("1000 input");
    expect(title).toContain("2000 cache read");
    expect(title).toContain("500 cache creation");
    expect(title).toContain("750 output");
  });

  it("renders a dash when the session has no token usage", () => {
    sessionsState.data = [rec("t-1", { usage: null })];

    render(<SessionList issueId="i-1" />);

    const list = screen.getByTestId("session-list");
    // Row has no element carrying a title (the tokens span sets the title
    // only when usage is present), so the cell renders the dash placeholder.
    expect(list.querySelector("[title]")).toBeNull();
    expect(within(list).getAllByText("—").length).toBeGreaterThan(0);
  });
});
