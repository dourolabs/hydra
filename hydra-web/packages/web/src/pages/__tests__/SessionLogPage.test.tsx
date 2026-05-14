import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, cleanup } from "@testing-library/react";
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
}));

vi.mock("../../layout/Breadcrumbs", () => ({
  Breadcrumbs: ({
    items,
    current,
  }: {
    items: { label: string; to: string }[];
    current: string;
  }) => (
    <nav data-testid="breadcrumbs">
      {items.map((item) => (
        <a key={item.to} href={item.to}>
          {item.label}
        </a>
      ))}
      <span data-testid="breadcrumb-current">{current}</span>
    </nav>
  ),
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
      prompt: "do the thing",
      context: { type: "none" },
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
});

afterEach(() => {
  cleanup();
});

describe("SessionLogPage", () => {
  it("renders breadcrumbs with Dashboard and Issue when issueId is in URL", () => {
    params.issueId = "i-1";
    params.sessionId = "t-1";
    sessionState.data = makeRecord("t-1");

    render(<SessionLogPage />);

    const breadcrumbs = screen.getByTestId("breadcrumbs");
    const links = breadcrumbs.querySelectorAll("a");
    expect(links).toHaveLength(2);
    expect(links[0].textContent).toBe("Dashboard");
    expect(links[0].getAttribute("href")).toBe("/");
    expect(links[1].textContent).toBe("Issue i-1");
    expect(links[1].getAttribute("href")).toBe("/issues/i-1");
    expect(screen.getByTestId("breadcrumb-current").textContent).toBe(
      "Session t-1",
    );
  });

  it("renders breadcrumbs with only Sessions when issueId is absent", () => {
    params.sessionId = "t-orphan";
    sessionState.data = makeRecord("t-orphan");

    render(<SessionLogPage />);

    const breadcrumbs = screen.getByTestId("breadcrumbs");
    const links = breadcrumbs.querySelectorAll("a");
    expect(links).toHaveLength(1);
    expect(links[0].textContent).toBe("Sessions");
    expect(links[0].getAttribute("href")).toBe("/sessions");
    expect(screen.getByTestId("breadcrumb-current").textContent).toBe(
      "Session t-orphan",
    );
  });

  it("omits the Issue meta link when issueId is absent", () => {
    params.sessionId = "t-orphan";
    sessionState.data = makeRecord("t-orphan");

    render(<SessionLogPage />);

    // No anchor pointing to /issues/* should be rendered in the meta header
    // (the breadcrumbs only contain the /sessions link).
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
    // Breadcrumb link + meta-header link.
    expect(issueLinks.length).toBeGreaterThanOrEqual(2);
  });
});
