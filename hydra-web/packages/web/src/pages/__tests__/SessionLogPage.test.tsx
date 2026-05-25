import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, cleanup } from "@testing-library/react";
import React from "react";
import type { SessionVersionRecord } from "@hydra/api";

// --- Mocks ---

const params: { issueId?: string; sessionId?: string } = {};

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
  useParams: () => params,
}));

interface SessionState {
  data: SessionVersionRecord | undefined;
  isLoading: boolean;
  error: Error | null;
}

const sessionState: SessionState = {
  data: undefined,
  isLoading: false,
  error: null,
};

vi.mock("../../features/sessions/useSession", () => ({
  useSession: () => ({
    data: sessionState.data,
    isLoading: sessionState.isLoading,
    error: sessionState.error,
  }),
}));

vi.mock("@tanstack/react-query", () => ({
  useMutation: () => ({
    mutate: vi.fn(),
    isPending: false,
  }),
  useQueryClient: () => ({
    cancelQueries: vi.fn(),
    getQueryData: vi.fn(),
    setQueryData: vi.fn(),
    invalidateQueries: vi.fn(),
  }),
}));

vi.mock("@hydra/ui", () => ({
  Avatar: ({ name }: { name: string }) => <span data-testid="avatar">{name}</span>,
  Badge: ({ status }: { status: string }) => (
    <span data-testid="badge">{status}</span>
  ),
  Button: ({
    children,
    onClick,
  }: {
    children: React.ReactNode;
    onClick?: () => void;
  }) => <button onClick={onClick}>{children}</button>,
  Spinner: () => <span data-testid="spinner" />,
  Tabs: ({
    tabs,
    activeTab,
  }: {
    tabs: { id: string; label: string }[];
    activeTab: string;
  }) => (
    <div data-testid="tabs">
      {tabs.map((t) => (
        <span key={t.id} data-active={t.id === activeTab}>
          {t.label}
        </span>
      ))}
    </div>
  ),
}));

vi.mock("../../features/sessions/SessionLogViewer", () => ({
  SessionLogViewer: () => <div data-testid="log-viewer" />,
}));

vi.mock("../../features/sessions/SessionSettings", () => ({
  SessionSettings: () => <div data-testid="session-settings" />,
}));

vi.mock("../../components/DeleteConfirmModal/DeleteConfirmModal", () => ({
  DeleteConfirmModal: () => null,
}));

vi.mock("../../api/client", () => ({
  apiClient: {
    killSession: vi.fn(),
  },
  ApiError: class ApiError extends Error {
    status: number;
    constructor(message: string, status: number) {
      super(message);
      this.status = status;
    }
  },
}));

vi.mock("../../features/toast/useToast", () => ({
  useToast: () => ({ addToast: vi.fn() }),
}));

vi.mock("../../utils/statusMapping", () => ({
  normalizeSessionStatus: (s: string) => s,
}));

vi.mock("../../utils/time", () => ({
  getRuntime: () => "1m",
  formatDuration: () => "1m",
  formatTimestamp: (s: string) => s,
  formatRelativeTime: (s: string) => s,
  shortRelativeTime: (s: string) => s,
}));

const useBreadcrumbsMock = vi.fn();
vi.mock("../../layout/useBreadcrumbs", () => ({
  useBreadcrumbs: (...args: unknown[]) => useBreadcrumbsMock(...args),
}));

vi.mock("../SessionLogPage.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

// --- Import after mocks ---
const { SessionLogPage } = await import("../SessionLogPage");

// --- Helpers ---

function makeRecord(
  sessionId: string,
  status: SessionVersionRecord["session"]["status"] = "running",
): SessionVersionRecord {
  return {
    session_id: sessionId,
    version: 1n,
    timestamp: "2026-03-15T10:00:00.000Z",
    session: {
      mode: { type: "headless", prompt: "do the thing" },
      agent_config: {},
      mount_spec: { working_dir: "repo", mounts: [] },
      creator: "swe",
      status,
      spawned_from: null,
      start_time: "2026-03-15T10:00:00.000Z",
      end_time: null,
      creation_time: "2026-03-15T10:00:00.000Z",
    },
  };
}

function reset() {
  params.issueId = undefined;
  params.sessionId = undefined;
  sessionState.data = undefined;
  sessionState.isLoading = false;
  sessionState.error = null;
}

beforeEach(() => {
  reset();
  useBreadcrumbsMock.mockReset();
});

afterEach(() => {
  cleanup();
});

describe("SessionLogPage", () => {
  it("publishes Workspace / Issues / issue-id breadcrumbs when issueId is in URL", () => {
    params.issueId = "i-1";
    params.sessionId = "t-1";
    sessionState.data = makeRecord("t-1");

    render(<SessionLogPage />);

    expect(useBreadcrumbsMock).toHaveBeenCalledWith(
      [
        { label: "Workspace", to: "/" },
        { label: "Issues", to: "/" },
        { label: "i-1", to: "/issues/i-1", kind: "code" },
      ],
      "t-1",
      "code",
    );
  });

  it("publishes Workspace / Sessions breadcrumbs when issueId is absent", () => {
    params.sessionId = "t-orphan";
    sessionState.data = makeRecord("t-orphan");

    render(<SessionLogPage />);

    expect(useBreadcrumbsMock).toHaveBeenCalledWith(
      [
        { label: "Workspace", to: "/" },
        { label: "Sessions", to: "/sessions" },
      ],
      "t-orphan",
      "code",
    );
  });

  it("omits the Issue meta link when issueId is absent", () => {
    params.sessionId = "t-orphan";
    sessionState.data = makeRecord("t-orphan");

    render(<SessionLogPage />);

    const anchors = Array.from(document.querySelectorAll("a"));
    const issueAnchors = anchors.filter((a) =>
      a.getAttribute("href")?.startsWith("/issues/"),
    );
    expect(issueAnchors).toHaveLength(0);
  });

  it("renders the Issue meta link when issueId is present", () => {
    params.issueId = "i-1";
    params.sessionId = "t-1";
    sessionState.data = makeRecord("t-1");

    render(<SessionLogPage />);

    const anchors = Array.from(document.querySelectorAll("a"));
    const issueLinks = anchors.filter(
      (a) => a.getAttribute("href") === "/issues/i-1",
    );
    expect(issueLinks.length).toBeGreaterThanOrEqual(1);
  });
});
