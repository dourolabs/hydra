// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, cleanup, act } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";

// --- Mocks ---

vi.mock("../../layout/useBreadcrumbs", () => ({
  useBreadcrumbs: () => undefined,
}));

vi.mock("../../api/client", () => ({
  apiClient: {
    listProjects: () =>
      Promise.resolve({
        projects: [
          {
            project_id: "j-defaul",
            version: 1,
            project: {
              key: "default",
              name: "Default",
              statuses: [],
              creator: "alice",
              deleted: false,
            },
          },
        ],
      }),
    getProjectStatuses: () =>
      Promise.resolve({
        statuses: [
          { key: "open", label: "Open", color: "#3498db" },
          { key: "closed", label: "Closed", color: "#2ecc71" },
        ],
      }),
    listRepositories: () =>
      Promise.resolve({
        repositories: [{ name: "dourolabs/hydra" }],
      }),
    getPatchesThroughputOverTime: vi.fn(() => Promise.resolve({ buckets: [] })),
    getPatchesThroughputTerminalMix: vi.fn(() =>
      Promise.resolve({ merged: BigInt(0), closed: BigInt(0) }),
    ),
    getPatchesThroughputTimeToMerge: vi.fn(() =>
      Promise.resolve({
        median_seconds: BigInt(0),
        p95_seconds: BigInt(0),
        count: BigInt(0),
        histogram: [],
      }),
    ),
    getPatchesThroughputInFlightOverTime: vi.fn(() => Promise.resolve({ buckets: [] })),
    getIssuesThroughputCycleTime: vi.fn(() =>
      Promise.resolve({
        median_seconds: BigInt(0),
        p95_seconds: BigInt(0),
        count: BigInt(0),
        histogram: [],
      }),
    ),
    getIssuesThroughputTimeInStatusBreakdown: vi.fn(() =>
      Promise.resolve({
        project_id: "j-defaul",
        status_segments: [],
        issue_count: BigInt(0),
      }),
    ),
    getIssuesThroughputPerStatusDistribution: vi.fn(() =>
      Promise.resolve({ project_id: "j-defaul", statuses: [] }),
    ),
    getIssuesThroughputOverTime: vi.fn(() => Promise.resolve({ buckets: [] })),
  },
}));

vi.mock("../AnalyticsThroughputPage.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("../../features/analytics/SlicerPanel.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("../../features/analytics/TimeRangePicker.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("../../features/analytics/ChartCard.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("@hydra/ui", () => ({
  Spinner: () => <span data-testid="spinner" />,
  Icons: new Proxy({}, { get: () => () => <span /> }),
  Panel: ({ children, header }: { children: React.ReactNode; header?: React.ReactNode }) => (
    <div data-testid="panel">
      {header !== undefined && <div data-testid="panel-header">{header}</div>}
      {children}
    </div>
  ),
  Select: ({
    label,
    options,
    placeholder,
    id,
    ...props
  }: {
    label?: string;
    placeholder?: string;
    id?: string;
    options: { value: string; label: string }[];
    [key: string]: unknown;
  }) => {
    const selectId = id ?? label?.toLowerCase().replace(/\s+/g, "-");
    return (
      <div>
        {label && <label htmlFor={selectId}>{label}</label>}
        <select id={selectId} {...(props as Record<string, unknown>)}>
          {placeholder && (
            <option value="" disabled>
              {placeholder}
            </option>
          )}
          {options.map((opt: { value: string; label: string }) => (
            <option key={opt.value} value={opt.value}>
              {opt.label}
            </option>
          ))}
        </select>
      </div>
    );
  },
  Input: ({ label, id, ...props }: { label?: string; id?: string; [key: string]: unknown }) => {
    const inputId = id ?? label?.toLowerCase().replace(/\s+/g, "-");
    return (
      <div>
        {label && <label htmlFor={inputId}>{label}</label>}
        <input id={inputId} {...(props as Record<string, unknown>)} />
      </div>
    );
  },
}));

function makeQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
}

const { AnalyticsThroughputPage } = await import("../AnalyticsThroughputPage");

function renderPage(initial = "/analytics/throughput") {
  return render(
    <QueryClientProvider client={makeQueryClient()}>
      <MemoryRouter initialEntries={[initial]}>
        <AnalyticsThroughputPage />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("AnalyticsThroughputPage", () => {
  beforeEach(() => {
    window.history.replaceState({}, "", "/analytics/throughput");
  });

  afterEach(() => {
    cleanup();
  });

  it("renders both chart sections with 4 placeholder cards each", () => {
    renderPage();
    expect(screen.getByTestId("analytics-patches-section")).toBeDefined();
    expect(screen.getByTestId("analytics-issues-section")).toBeDefined();
    expect(screen.getByTestId("chart-patches-over-time")).toBeDefined();
    expect(screen.getByTestId("chart-patches-terminal-mix")).toBeDefined();
    expect(screen.getByTestId("chart-patches-time-to-merge")).toBeDefined();
    expect(screen.getByTestId("chart-patches-in-flight")).toBeDefined();
    expect(screen.getByTestId("chart-issues-over-time")).toBeDefined();
    expect(screen.getByTestId("chart-issues-cycle-time")).toBeDefined();
    expect(screen.getByTestId("chart-issues-time-in-status")).toBeDefined();
    expect(screen.getByTestId("chart-issues-per-status")).toBeDefined();
  });

  it("renders the time-range picker with the default range highlighted", () => {
    renderPage();
    const button = screen.getByTestId("time-range-30d");
    expect(button.getAttribute("aria-pressed")).toBe("true");
  });

  it("disables the project-scoped issues cards until a project is picked", () => {
    renderPage();
    const card = screen.getByTestId("chart-issues-time-in-status");
    expect(card.textContent).toContain("Select a project");
  });

  it("renders the slicer panel", () => {
    renderPage();
    expect(screen.getByTestId("slicer-panel")).toBeDefined();
    expect(screen.getByTestId("slicer-project")).toBeDefined();
    expect(screen.getByTestId("slicer-repo")).toBeDefined();
    expect(screen.getByTestId("slicer-issue-type")).toBeDefined();
  });

  it("reads existing range from the URL", () => {
    renderPage("/analytics/throughput?range=7d");
    expect(screen.getByTestId("time-range-7d").getAttribute("aria-pressed")).toBe("true");
  });

  it("clicking a time-range button updates the active selection", () => {
    renderPage();
    act(() => {
      screen.getByTestId("time-range-90d").click();
    });
    expect(screen.getByTestId("time-range-90d").getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByTestId("time-range-30d").getAttribute("aria-pressed")).toBe("false");
  });
});
