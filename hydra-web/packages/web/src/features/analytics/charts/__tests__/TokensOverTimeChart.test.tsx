// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, cleanup } from "@testing-library/react";
import type { ReactNode } from "react";
import type { UseQueryResult } from "@tanstack/react-query";
import type { TokenUsageOverTimeQuery, TokenUsageOverTimeResponse } from "@hydra/api";

vi.mock("recharts", () => {
  const Passthrough = ({ children }: { children?: ReactNode }) => (
    <div data-testid="recharts-mock">{children}</div>
  );
  const Noop = () => null;
  return {
    ResponsiveContainer: Passthrough,
    AreaChart: Passthrough,
    Area: Noop,
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
  useTokenUsageOverTime: vi.fn(),
}));

vi.mock("../../useTokenUsage", () => hookMocks);

const { TokensOverTimeChart } = await import("../index");

const baseQuery: TokenUsageOverTimeQuery = {
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

describe("TokensOverTimeChart", () => {
  it("renders the empty state when buckets is empty", () => {
    hookMocks.useTokenUsageOverTime.mockReturnValue(
      mkResult<TokenUsageOverTimeResponse>({ data: { buckets: [] } }),
    );
    render(<TokensOverTimeChart query={baseQuery} />);
    expect(screen.getByText("No data in this window")).toBeDefined();
    expect(screen.queryByTestId("tokens-over-time-content")).toBeNull();
  });

  it("renders the chart + 4 legend entries when data is present", () => {
    hookMocks.useTokenUsageOverTime.mockReturnValue(
      mkResult<TokenUsageOverTimeResponse>({
        data: {
          buckets: [
            {
              bucket_start: "2026-05-10T00:00:00Z",
              input_tokens: BigInt(1000),
              output_tokens: BigInt(400),
              cache_read_input_tokens: BigInt(2000),
              cache_creation_input_tokens: BigInt(150),
            },
          ],
        },
      }),
    );
    render(<TokensOverTimeChart query={baseQuery} />);
    expect(screen.getByTestId("tokens-over-time-content")).toBeDefined();
    expect(screen.getByTestId("tokens-over-time-legend-input_tokens")).toBeDefined();
    expect(screen.getByTestId("tokens-over-time-legend-output_tokens")).toBeDefined();
    expect(screen.getByTestId("tokens-over-time-legend-cache_read_input_tokens")).toBeDefined();
    expect(screen.getByTestId("tokens-over-time-legend-cache_creation_input_tokens")).toBeDefined();
  });

  it("renders the error state via ChartCard", () => {
    hookMocks.useTokenUsageOverTime.mockReturnValue(
      mkResult<TokenUsageOverTimeResponse>({
        error: new Error("boom"),
        isError: true,
        isSuccess: false,
        status: "error",
      }),
    );
    render(<TokensOverTimeChart query={baseQuery} />);
    expect(screen.getByTestId("chart-card-error").textContent).toContain("boom");
  });

  it("renders the loading state via ChartCard", () => {
    hookMocks.useTokenUsageOverTime.mockReturnValue(
      mkResult<TokenUsageOverTimeResponse>({
        isLoading: true,
        isPending: true,
        isSuccess: false,
        status: "pending",
        fetchStatus: "fetching",
      }),
    );
    render(<TokensOverTimeChart query={baseQuery} />);
    expect(screen.getByTestId("chart-card-loading")).toBeDefined();
  });
});
