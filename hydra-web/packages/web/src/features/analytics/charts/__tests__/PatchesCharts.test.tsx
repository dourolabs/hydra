// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, cleanup } from "@testing-library/react";
import type { ReactNode } from "react";
import type { UseQueryResult } from "@tanstack/react-query";
import type {
  PatchesOverTimeResponse,
  PatchesTerminalMixResponse,
  PatchesTimeToMergeResponse,
  PatchesInFlightOverTimeResponse,
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
  useThroughputPatchesOverTime: vi.fn(),
  useThroughputPatchesTerminalMix: vi.fn(),
  useThroughputPatchesTimeToMerge: vi.fn(),
  useThroughputPatchesInFlightOverTime: vi.fn(),
}));

vi.mock("../../useThroughputPatches", () => hookMocks);

const {
  PatchesOverTimeChart,
  PatchesTerminalMixChart,
  PatchesTimeToMergeChart,
  PatchesInFlightChart,
} = await import("../index");

import type { PatchesThroughputQuery } from "@hydra/api";

const baseQuery: PatchesThroughputQuery = {
  from: "2026-05-10T00:00:00Z",
  to: "2026-06-10T00:00:00Z",
  bucket: "day",
  repo_name: null,
  creator: null,
};

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

describe("PatchesOverTimeChart", () => {
  it("renders the empty state when buckets is empty", () => {
    hookMocks.useThroughputPatchesOverTime.mockReturnValue(
      mkResult<PatchesOverTimeResponse>({ data: { buckets: [] } }),
    );
    render(<PatchesOverTimeChart query={baseQuery} />);
    expect(screen.getByText("No data in this window")).toBeDefined();
    expect(screen.queryByTestId("patches-over-time-content")).toBeNull();
  });

  it("renders the chart + legend when data is present", () => {
    hookMocks.useThroughputPatchesOverTime.mockReturnValue(
      mkResult<PatchesOverTimeResponse>({
        data: {
          buckets: [
            { bucket_start: "2026-05-10T00:00:00Z", created: BigInt(3), merged: BigInt(2) },
            { bucket_start: "2026-05-11T00:00:00Z", created: BigInt(5), merged: BigInt(4) },
          ],
        },
      }),
    );
    render(<PatchesOverTimeChart query={baseQuery} />);
    expect(screen.getByTestId("patches-over-time-content")).toBeDefined();
    expect(screen.getByText("Created")).toBeDefined();
    expect(screen.getByText("Merged")).toBeDefined();
  });

  it("renders the error state via ChartCard", () => {
    hookMocks.useThroughputPatchesOverTime.mockReturnValue(
      mkResult<PatchesOverTimeResponse>({
        error: new Error("boom"),
        isError: true,
        isSuccess: false,
        status: "error",
      }),
    );
    render(<PatchesOverTimeChart query={baseQuery} />);
    expect(screen.getByTestId("chart-card-error").textContent).toContain("boom");
  });

  it("renders the loading state via ChartCard", () => {
    hookMocks.useThroughputPatchesOverTime.mockReturnValue(
      mkResult<PatchesOverTimeResponse>({
        isLoading: true,
        isPending: true,
        isSuccess: false,
        status: "pending",
        fetchStatus: "fetching",
      }),
    );
    render(<PatchesOverTimeChart query={baseQuery} />);
    expect(screen.getByTestId("chart-card-loading")).toBeDefined();
  });
});

describe("PatchesTerminalMixChart", () => {
  it("renders the empty state when both counts are zero", () => {
    hookMocks.useThroughputPatchesTerminalMix.mockReturnValue(
      mkResult<PatchesTerminalMixResponse>({ data: { merged: BigInt(0), closed: BigInt(0) } }),
    );
    render(<PatchesTerminalMixChart query={baseQuery} />);
    expect(screen.getByText("No data in this window")).toBeDefined();
  });

  it("renders the donut total + percentages with data", () => {
    hookMocks.useThroughputPatchesTerminalMix.mockReturnValue(
      mkResult<PatchesTerminalMixResponse>({ data: { merged: BigInt(27), closed: BigInt(4) } }),
    );
    render(<PatchesTerminalMixChart query={baseQuery} />);
    expect(screen.getByTestId("patches-terminal-mix-total").textContent).toBe("31");
    expect(screen.getByText(/Merged: 27 \(87%\)/)).toBeDefined();
    expect(screen.getByText(/Closed: 4 \(13%\)/)).toBeDefined();
  });

  it("renders a total even when one side is 0", () => {
    hookMocks.useThroughputPatchesTerminalMix.mockReturnValue(
      mkResult<PatchesTerminalMixResponse>({ data: { merged: BigInt(5), closed: BigInt(0) } }),
    );
    render(<PatchesTerminalMixChart query={baseQuery} />);
    expect(screen.getByTestId("patches-terminal-mix-total").textContent).toBe("5");
    expect(screen.getByText(/Merged: 5 \(100%\)/)).toBeDefined();
    expect(screen.getByText(/Closed: 0 \(0%\)/)).toBeDefined();
  });

  it("renders the error state via ChartCard", () => {
    hookMocks.useThroughputPatchesTerminalMix.mockReturnValue(
      mkResult<PatchesTerminalMixResponse>({
        error: new Error("nope"),
        isError: true,
        isSuccess: false,
        status: "error",
      }),
    );
    render(<PatchesTerminalMixChart query={baseQuery} />);
    expect(screen.getByTestId("chart-card-error").textContent).toContain("nope");
  });
});

describe("PatchesTimeToMergeChart", () => {
  it("renders the empty state when count is zero", () => {
    hookMocks.useThroughputPatchesTimeToMerge.mockReturnValue(
      mkResult<PatchesTimeToMergeResponse>({
        data: {
          median_seconds: null,
          p95_seconds: null,
          count: BigInt(0),
          histogram: [],
        },
      }),
    );
    render(<PatchesTimeToMergeChart query={baseQuery} />);
    expect(screen.getByText("No data in this window")).toBeDefined();
  });

  it("renders the histogram and formatted callouts", () => {
    hookMocks.useThroughputPatchesTimeToMerge.mockReturnValue(
      mkResult<PatchesTimeToMergeResponse>({
        data: {
          median_seconds: BigInt(18000),
          p95_seconds: BigInt(86400),
          count: BigInt(7),
          histogram: [
            { bin_start_seconds: BigInt(0), bin_end_seconds: BigInt(3600), count: BigInt(1) },
            { bin_start_seconds: BigInt(86400 * 30), bin_end_seconds: null, count: BigInt(1) },
          ],
        },
      }),
    );
    render(<PatchesTimeToMergeChart query={baseQuery} />);
    expect(screen.getByTestId("patches-time-to-merge-content")).toBeDefined();
    const callouts = screen.getByTestId("patches-time-to-merge-callouts");
    // 18000s = 5h, 86400s = 1d, count = 7
    expect(callouts.textContent).toContain("5h");
    expect(callouts.textContent).toContain("1d");
    expect(callouts.textContent).toContain("7");
  });

  it("renders a dash for median/p95 callouts when they are null but count > 0", () => {
    hookMocks.useThroughputPatchesTimeToMerge.mockReturnValue(
      mkResult<PatchesTimeToMergeResponse>({
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
    render(<PatchesTimeToMergeChart query={baseQuery} />);
    const callouts = screen.getByTestId("patches-time-to-merge-callouts");
    expect(callouts.textContent).toContain("—");
  });

  it("renders the error state via ChartCard", () => {
    hookMocks.useThroughputPatchesTimeToMerge.mockReturnValue(
      mkResult<PatchesTimeToMergeResponse>({
        error: new Error("bad"),
        isError: true,
        isSuccess: false,
        status: "error",
      }),
    );
    render(<PatchesTimeToMergeChart query={baseQuery} />);
    expect(screen.getByTestId("chart-card-error").textContent).toContain("bad");
  });
});

describe("PatchesInFlightChart", () => {
  it("renders the empty state when buckets is empty", () => {
    hookMocks.useThroughputPatchesInFlightOverTime.mockReturnValue(
      mkResult<PatchesInFlightOverTimeResponse>({ data: { buckets: [] } }),
    );
    render(<PatchesInFlightChart query={baseQuery} />);
    expect(screen.getByText("No data in this window")).toBeDefined();
  });

  it("renders the chart content with data", () => {
    hookMocks.useThroughputPatchesInFlightOverTime.mockReturnValue(
      mkResult<PatchesInFlightOverTimeResponse>({
        data: {
          buckets: [
            { bucket_start: "2026-05-10T00:00:00Z", in_flight: BigInt(12) },
            { bucket_start: "2026-05-11T00:00:00Z", in_flight: BigInt(14) },
          ],
        },
      }),
    );
    render(<PatchesInFlightChart query={baseQuery} />);
    expect(screen.getByTestId("patches-in-flight-content")).toBeDefined();
  });

  it("renders the error state via ChartCard", () => {
    hookMocks.useThroughputPatchesInFlightOverTime.mockReturnValue(
      mkResult<PatchesInFlightOverTimeResponse>({
        error: new Error("kaboom"),
        isError: true,
        isSuccess: false,
        status: "error",
      }),
    );
    render(<PatchesInFlightChart query={baseQuery} />);
    expect(screen.getByTestId("chart-card-error").textContent).toContain("kaboom");
  });
});
