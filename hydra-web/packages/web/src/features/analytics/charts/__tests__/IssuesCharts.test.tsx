// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, cleanup } from "@testing-library/react";
import type { ReactNode } from "react";
import type { UseQueryResult } from "@tanstack/react-query";
import type {
  IssuesOverTimeResponse,
  IssuesCycleTimeResponse,
  IssuesTimeInStatusBreakdownResponse,
  IssuesPerStatusDistributionResponse,
} from "@hydra/api";

// Mock recharts so we don't depend on SVG layout in jsdom — we only assert
// our own composition (callouts, legend, empty/error/loading states).
vi.mock("recharts", () => {
  const Passthrough = ({ children }: { children?: ReactNode }) => (
    <div data-testid="recharts-mock">{children}</div>
  );
  const Noop = () => null;
  return {
    ResponsiveContainer: Passthrough,
    AreaChart: Passthrough,
    BarChart: Passthrough,
    LineChart: Passthrough,
    PieChart: Passthrough,
    Pie: Passthrough,
    Cell: Noop,
    Area: Noop,
    Bar: Noop,
    Line: Noop,
    XAxis: Noop,
    YAxis: Noop,
    CartesianGrid: Noop,
    Tooltip: Noop,
  };
});

vi.mock("@hydra/ui", () => ({
  Spinner: () => <span data-testid="spinner" />,
  Panel: ({ children, header }: { children: ReactNode; header?: ReactNode }) => (
    <div data-testid="panel">
      {header !== undefined && <div data-testid="panel-header">{header}</div>}
      {children}
    </div>
  ),
}));

vi.mock("../charts.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("../../ChartCard.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const hookMocks = vi.hoisted(() => ({
  useThroughputIssuesOverTime: vi.fn(),
  useThroughputIssuesCycleTime: vi.fn(),
  useThroughputIssuesTimeInStatusBreakdown: vi.fn(),
  useThroughputIssuesPerStatusDistribution: vi.fn(),
}));

vi.mock("../../useThroughputIssues", () => hookMocks);

const {
  IssuesOverTimeChart,
  IssuesCycleTimeChart,
  IssuesTimeInStatusBreakdownChart,
  IssuesPerStatusDistributionChart,
} = await import("../index");

import type { IssuesThroughputQuery } from "@hydra/api";

const baseQuery: IssuesThroughputQuery = {
  from: "2026-05-10T00:00:00Z",
  to: "2026-06-10T00:00:00Z",
  bucket: "day",
  project_id: null,
  repo_name: null,
  issue_type: null,
  assignee: null,
  creator: null,
};

const scopedQuery: IssuesThroughputQuery = { ...baseQuery, project_id: "j-defaul" };

function mkResult<T>(partial: Partial<UseQueryResult<T>>): UseQueryResult<T> {
  return {
    data: undefined,
    error: null,
    isLoading: false,
    isPending: false,
    isSuccess: true,
    isError: false,
    status: "success",
    fetchStatus: "idle",
    ...partial,
  } as unknown as UseQueryResult<T>;
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("IssuesOverTimeChart", () => {
  it("renders the empty state when buckets is empty", () => {
    hookMocks.useThroughputIssuesOverTime.mockReturnValue(
      mkResult<IssuesOverTimeResponse>({ data: { buckets: [] } }),
    );
    render(<IssuesOverTimeChart query={baseQuery} />);
    expect(screen.getByText("No data in this window")).toBeDefined();
    expect(screen.queryByTestId("issues-over-time-content")).toBeNull();
  });

  it("renders the chart + legend when data is present", () => {
    hookMocks.useThroughputIssuesOverTime.mockReturnValue(
      mkResult<IssuesOverTimeResponse>({
        data: {
          buckets: [
            {
              bucket_start: "2026-05-10T00:00:00Z",
              created: BigInt(4),
              reached_terminal: BigInt(2),
            },
            {
              bucket_start: "2026-05-11T00:00:00Z",
              created: BigInt(5),
              reached_terminal: BigInt(3),
            },
          ],
        },
      }),
    );
    render(<IssuesOverTimeChart query={baseQuery} />);
    expect(screen.getByTestId("issues-over-time-content")).toBeDefined();
    expect(screen.getByText("Reached terminal")).toBeDefined();
    expect(screen.getByText("Total")).toBeDefined();
  });

  it("renders the error state via ChartCard", () => {
    hookMocks.useThroughputIssuesOverTime.mockReturnValue(
      mkResult<IssuesOverTimeResponse>({
        error: new Error("boom"),
        isError: true,
        isSuccess: false,
        status: "error",
      }),
    );
    render(<IssuesOverTimeChart query={baseQuery} />);
    expect(screen.getByTestId("chart-card-error").textContent).toContain("boom");
  });

  it("renders the loading state via ChartCard", () => {
    hookMocks.useThroughputIssuesOverTime.mockReturnValue(
      mkResult<IssuesOverTimeResponse>({
        isLoading: true,
        isPending: true,
        isSuccess: false,
        status: "pending",
        fetchStatus: "fetching",
      }),
    );
    render(<IssuesOverTimeChart query={baseQuery} />);
    expect(screen.getByTestId("chart-card-loading")).toBeDefined();
  });

  it("sets a region landmark with the chart title as the aria-label", () => {
    hookMocks.useThroughputIssuesOverTime.mockReturnValue(
      mkResult<IssuesOverTimeResponse>({ data: { buckets: [] } }),
    );
    render(<IssuesOverTimeChart query={baseQuery} />);
    const card = screen.getByTestId("chart-issues-over-time");
    expect(card.getAttribute("role")).toBe("region");
    expect(card.getAttribute("aria-label")).toBe("Issues over time");
  });
});

describe("IssuesCycleTimeChart", () => {
  it("renders the empty state when count is zero", () => {
    hookMocks.useThroughputIssuesCycleTime.mockReturnValue(
      mkResult<IssuesCycleTimeResponse>({
        data: {
          median_seconds: null,
          p95_seconds: null,
          count: BigInt(0),
          histogram: [],
        },
      }),
    );
    render(<IssuesCycleTimeChart query={baseQuery} />);
    expect(screen.getByText("No data in this window")).toBeDefined();
  });

  it("renders the histogram and formatted callouts", () => {
    hookMocks.useThroughputIssuesCycleTime.mockReturnValue(
      mkResult<IssuesCycleTimeResponse>({
        data: {
          median_seconds: BigInt(86400),
          p95_seconds: BigInt(604800),
          count: BigInt(9),
          histogram: [
            { bin_start_seconds: BigInt(0), bin_end_seconds: BigInt(3600), count: BigInt(1) },
            { bin_start_seconds: BigInt(86400 * 30), bin_end_seconds: null, count: BigInt(1) },
          ],
        },
      }),
    );
    render(<IssuesCycleTimeChart query={baseQuery} />);
    expect(screen.getByTestId("issues-cycle-time-content")).toBeDefined();
    const callouts = screen.getByTestId("issues-cycle-time-callouts");
    // 86400s = 1d, 604800s = 7d, count = 9
    expect(callouts.textContent).toContain("1d");
    expect(callouts.textContent).toContain("7d");
    expect(callouts.textContent).toContain("9");
  });

  it("renders a dash for median/p95 callouts when they are null but count > 0", () => {
    hookMocks.useThroughputIssuesCycleTime.mockReturnValue(
      mkResult<IssuesCycleTimeResponse>({
        data: {
          median_seconds: null,
          p95_seconds: null,
          count: BigInt(1),
          histogram: [
            { bin_start_seconds: BigInt(0), bin_end_seconds: BigInt(3600), count: BigInt(1) },
          ],
        },
      }),
    );
    render(<IssuesCycleTimeChart query={baseQuery} />);
    const callouts = screen.getByTestId("issues-cycle-time-callouts");
    expect(callouts.textContent).toContain("—");
  });

  it("renders the error state via ChartCard", () => {
    hookMocks.useThroughputIssuesCycleTime.mockReturnValue(
      mkResult<IssuesCycleTimeResponse>({
        error: new Error("bad"),
        isError: true,
        isSuccess: false,
        status: "error",
      }),
    );
    render(<IssuesCycleTimeChart query={baseQuery} />);
    expect(screen.getByTestId("chart-card-error").textContent).toContain("bad");
  });
});

describe("IssuesTimeInStatusBreakdownChart", () => {
  it("renders the disabled placeholder when no project is selected", () => {
    hookMocks.useThroughputIssuesTimeInStatusBreakdown.mockReturnValue(
      mkResult<IssuesTimeInStatusBreakdownResponse>({}),
    );
    render(<IssuesTimeInStatusBreakdownChart query={baseQuery} hasProject={false} />);
    expect(screen.getByTestId("chart-card-disabled").textContent).toContain("Select a project");
    // The hook is passed `enabled: hasProject = false`.
    expect(hookMocks.useThroughputIssuesTimeInStatusBreakdown).toHaveBeenCalledWith(
      expect.any(Object),
      false,
    );
  });

  it("renders segments + legend in the order returned by the backend", () => {
    hookMocks.useThroughputIssuesTimeInStatusBreakdown.mockReturnValue(
      mkResult<IssuesTimeInStatusBreakdownResponse>({
        data: {
          project_id: "j-defaul",
          issue_count: BigInt(12),
          status_segments: [
            { status_key: "open", label: "Open", color: "#3498db", mean_seconds: BigInt(1200) },
            {
              status_key: "in-progress",
              label: "In progress",
              color: "#f1c40f",
              mean_seconds: BigInt(21600),
            },
            { status_key: "closed", label: "Closed", color: "#2ecc71", mean_seconds: BigInt(0) },
          ],
        },
      }),
    );
    render(<IssuesTimeInStatusBreakdownChart query={scopedQuery} hasProject={true} />);
    expect(screen.getByTestId("issues-time-in-status-content")).toBeDefined();
    // The first two render; the third has 0% width and is skipped from the bar.
    expect(screen.getByTestId("issues-time-in-status-segment-open")).toBeDefined();
    expect(screen.getByTestId("issues-time-in-status-segment-in-progress")).toBeDefined();
    expect(screen.queryByTestId("issues-time-in-status-segment-closed")).toBeNull();
    // Legend keeps all statuses, including the 0-time terminal.
    expect(screen.getByTestId("issues-time-in-status-legend-open")).toBeDefined();
    expect(screen.getByTestId("issues-time-in-status-legend-in-progress")).toBeDefined();
    expect(screen.getByTestId("issues-time-in-status-legend-closed")).toBeDefined();
  });

  it("uses backend status colors verbatim on the segments", () => {
    hookMocks.useThroughputIssuesTimeInStatusBreakdown.mockReturnValue(
      mkResult<IssuesTimeInStatusBreakdownResponse>({
        data: {
          project_id: "j-defaul",
          issue_count: BigInt(1),
          status_segments: [
            { status_key: "open", label: "Open", color: "#abcdef", mean_seconds: BigInt(60) },
          ],
        },
      }),
    );
    render(<IssuesTimeInStatusBreakdownChart query={scopedQuery} hasProject={true} />);
    const seg = screen.getByTestId("issues-time-in-status-segment-open") as HTMLElement;
    // jsdom may keep the literal hex or normalize to rgb(); accept both.
    const bg = seg.style.background.toLowerCase();
    expect(bg.includes("#abcdef") || bg.includes("rgb(171, 205, 239)")).toBe(true);
  });

  it("renders the empty state when issue_count is zero", () => {
    hookMocks.useThroughputIssuesTimeInStatusBreakdown.mockReturnValue(
      mkResult<IssuesTimeInStatusBreakdownResponse>({
        data: { project_id: "j-defaul", issue_count: BigInt(0), status_segments: [] },
      }),
    );
    render(<IssuesTimeInStatusBreakdownChart query={scopedQuery} hasProject={true} />);
    expect(screen.getByText("No data in this window")).toBeDefined();
  });

  it("renders the error state via ChartCard", () => {
    hookMocks.useThroughputIssuesTimeInStatusBreakdown.mockReturnValue(
      mkResult<IssuesTimeInStatusBreakdownResponse>({
        error: new Error("kaboom"),
        isError: true,
        isSuccess: false,
        status: "error",
      }),
    );
    render(<IssuesTimeInStatusBreakdownChart query={scopedQuery} hasProject={true} />);
    expect(screen.getByTestId("chart-card-error").textContent).toContain("kaboom");
  });
});

describe("IssuesPerStatusDistributionChart", () => {
  it("renders the disabled placeholder when no project is selected", () => {
    hookMocks.useThroughputIssuesPerStatusDistribution.mockReturnValue(
      mkResult<IssuesPerStatusDistributionResponse>({}),
    );
    render(<IssuesPerStatusDistributionChart query={baseQuery} hasProject={false} />);
    expect(screen.getByTestId("chart-card-disabled").textContent).toContain("Select a project");
    expect(hookMocks.useThroughputIssuesPerStatusDistribution).toHaveBeenCalledWith(
      expect.any(Object),
      false,
    );
  });

  it("renders a card per status with formatted callouts", () => {
    hookMocks.useThroughputIssuesPerStatusDistribution.mockReturnValue(
      mkResult<IssuesPerStatusDistributionResponse>({
        data: {
          project_id: "j-defaul",
          statuses: [
            {
              status_key: "in-progress",
              label: "In progress",
              color: "#f1c40f",
              median_seconds: BigInt(18000),
              p95_seconds: BigInt(86400),
              sample_count: BigInt(8),
            },
            {
              status_key: "closed",
              label: "Closed",
              color: "#2ecc71",
              median_seconds: null,
              p95_seconds: null,
              sample_count: BigInt(0),
            },
          ],
        },
      }),
    );
    render(<IssuesPerStatusDistributionChart query={scopedQuery} hasProject={true} />);
    expect(screen.getByTestId("issues-per-status-content")).toBeDefined();
    const inProgress = screen.getByTestId("issues-per-status-card-in-progress");
    // 18000s = 5h, 86400s = 1d
    expect(inProgress.textContent).toContain("5h");
    expect(inProgress.textContent).toContain("1d");
    expect(inProgress.textContent).toContain("8");
    const closed = screen.getByTestId("issues-per-status-card-closed");
    // 0 samples → median/p95 dashed
    expect(closed.textContent).toContain("—");
  });

  it("renders the empty state when statuses is empty", () => {
    hookMocks.useThroughputIssuesPerStatusDistribution.mockReturnValue(
      mkResult<IssuesPerStatusDistributionResponse>({
        data: { project_id: "j-defaul", statuses: [] },
      }),
    );
    render(<IssuesPerStatusDistributionChart query={scopedQuery} hasProject={true} />);
    expect(screen.getByText("No data in this window")).toBeDefined();
  });

  it("renders the error state via ChartCard", () => {
    hookMocks.useThroughputIssuesPerStatusDistribution.mockReturnValue(
      mkResult<IssuesPerStatusDistributionResponse>({
        error: new Error("nope"),
        isError: true,
        isSuccess: false,
        status: "error",
      }),
    );
    render(<IssuesPerStatusDistributionChart query={scopedQuery} hasProject={true} />);
    expect(screen.getByTestId("chart-card-error").textContent).toContain("nope");
  });
});
