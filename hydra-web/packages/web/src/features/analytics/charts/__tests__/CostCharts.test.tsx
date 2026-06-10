// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, cleanup } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";
import type { UseQueryResult } from "@tanstack/react-query";
import type {
  TokenUsageCostPerAgentResponse,
  TokenUsageQuery,
  TokenUsageTopIssuesByCostResponse,
} from "@hydra/api";

vi.mock("recharts", () => {
  const Passthrough = ({ children }: { children?: ReactNode }) => (
    <div data-testid="recharts-mock">{children}</div>
  );
  const Noop = () => null;
  return {
    ResponsiveContainer: Passthrough,
    BarChart: Passthrough,
    Bar: Noop,
    ScatterChart: Passthrough,
    Scatter: Noop,
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
  useTokenUsageCostPerAgent: vi.fn(),
  useTokenUsageTopIssuesByCost: vi.fn(),
  useTokenUsageOverTime: vi.fn(),
}));

vi.mock("../../useTokenUsage", () => hookMocks);

const { CostPerAgentChart, PerSessionCostScatterChart, TopIssuesByCostList } =
  await import("../index");
const { sessionJitter, agentDisplayName, formatUsd } = await import("../cost");

const baseQuery: TokenUsageQuery = {
  from: "2026-05-10T00:00:00Z",
  to: "2026-06-10T00:00:00Z",
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

describe("cost helpers", () => {
  it("formatUsd renders 2-decimal USD", () => {
    expect(formatUsd(1234.5)).toBe("$1,234.50");
    expect(formatUsd(0)).toBe("$0.00");
  });

  it("agentDisplayName maps null to the Ad-hoc bucket label", () => {
    expect(agentDisplayName(null)).toBe("Ad-hoc");
    expect(agentDisplayName("swe")).toBe("swe");
  });

  it("sessionJitter is deterministic and stays within [-0.4, +0.4]", () => {
    for (const id of ["s-a", "s-b", "s-very-long-session-id-0001", "s-z9"]) {
      const a = sessionJitter(id);
      const b = sessionJitter(id);
      expect(a).toBe(b);
      expect(a).toBeGreaterThanOrEqual(-0.4);
      expect(a).toBeLessThanOrEqual(0.4);
    }
  });
});

describe("CostPerAgentChart", () => {
  it("renders the empty state when agents is empty", () => {
    hookMocks.useTokenUsageCostPerAgent.mockReturnValue(
      mkResult<TokenUsageCostPerAgentResponse>({ data: { agents: [] } }),
    );
    render(<CostPerAgentChart query={baseQuery} />);
    expect(screen.getByText("No data in this window")).toBeDefined();
    expect(screen.queryByTestId("cost-per-agent-content")).toBeNull();
  });

  it("renders chart content when agents are present", () => {
    hookMocks.useTokenUsageCostPerAgent.mockReturnValue(
      mkResult<TokenUsageCostPerAgentResponse>({
        data: {
          agents: [
            {
              agent_name: "swe",
              total_cost_usd: 84.21,
              sessions: [{ session_id: "s-1", cost_usd: 42.1 }],
            },
            { agent_name: null, total_cost_usd: 12.4, sessions: [] },
          ],
        },
      }),
    );
    render(<CostPerAgentChart query={baseQuery} />);
    expect(screen.getByTestId("cost-per-agent-content")).toBeDefined();
  });

  it("renders the loading state via ChartCard", () => {
    hookMocks.useTokenUsageCostPerAgent.mockReturnValue(
      mkResult<TokenUsageCostPerAgentResponse>({
        isLoading: true,
        isPending: true,
        isSuccess: false,
        status: "pending",
        fetchStatus: "fetching",
      }),
    );
    render(<CostPerAgentChart query={baseQuery} />);
    expect(screen.getByTestId("chart-card-loading")).toBeDefined();
  });
});

describe("PerSessionCostScatterChart", () => {
  it("renders the empty state when agents is empty", () => {
    hookMocks.useTokenUsageCostPerAgent.mockReturnValue(
      mkResult<TokenUsageCostPerAgentResponse>({ data: { agents: [] } }),
    );
    render(<PerSessionCostScatterChart query={baseQuery} />);
    expect(screen.getByText("No data in this window")).toBeDefined();
  });

  it("renders chart content when sessions are present", () => {
    hookMocks.useTokenUsageCostPerAgent.mockReturnValue(
      mkResult<TokenUsageCostPerAgentResponse>({
        data: {
          agents: [
            {
              agent_name: "swe",
              total_cost_usd: 84.21,
              sessions: [
                { session_id: "s-1", cost_usd: 42.1 },
                { session_id: "s-2", cost_usd: 42.11 },
              ],
            },
            { agent_name: null, total_cost_usd: 0, sessions: [] },
          ],
        },
      }),
    );
    render(<PerSessionCostScatterChart query={baseQuery} />);
    expect(screen.getByTestId("per-session-cost-content")).toBeDefined();
  });
});

describe("TopIssuesByCostList", () => {
  it("renders the empty state when issues is empty", () => {
    hookMocks.useTokenUsageTopIssuesByCost.mockReturnValue(
      mkResult<TokenUsageTopIssuesByCostResponse>({ data: { issues: [] } }),
    );
    render(
      <MemoryRouter>
        <TopIssuesByCostList query={baseQuery} />
      </MemoryRouter>,
    );
    expect(screen.getByText("No data in this window")).toBeDefined();
  });

  it("renders rows in given order with /issues/<id> links and singular/plural counts", () => {
    hookMocks.useTokenUsageTopIssuesByCost.mockReturnValue(
      mkResult<TokenUsageTopIssuesByCostResponse>({
        data: {
          issues: [
            {
              issue_id: "i-aaa1111",
              title: "First",
              cost_usd: 12.5,
              session_count: BigInt(1),
            },
            {
              issue_id: "i-bbb2222",
              title: "Second",
              cost_usd: 4.0,
              session_count: BigInt(3),
            },
          ],
        },
      }),
    );
    render(
      <MemoryRouter>
        <TopIssuesByCostList query={baseQuery} />
      </MemoryRouter>,
    );
    expect(screen.getByText("$12.50")).toBeDefined();
    expect(screen.getByText("$4.00")).toBeDefined();
    expect(screen.getByText("1 session")).toBeDefined();
    expect(screen.getByText("3 sessions")).toBeDefined();
    const first = screen.getByRole("link", { name: "First" });
    expect(first.getAttribute("href")).toBe("/issues/i-aaa1111");
    const second = screen.getByRole("link", { name: "Second" });
    expect(second.getAttribute("href")).toBe("/issues/i-bbb2222");
  });
});
