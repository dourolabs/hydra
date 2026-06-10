import { useMemo } from "react";
import {
  Bar,
  BarChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import type {
  TokenUsageCostPerAgentResponse,
  TokenUsageQuery,
} from "@hydra/api";
import { ChartCard } from "../ChartCard";
import { useTokenUsageCostPerAgent } from "../useTokenUsage";
import { agentDisplayName, formatUsd } from "./cost";
import { AXIS_TICK, CHART_COLORS, GRID_STROKE, TOOLTIP_STYLE } from "./colors";
import styles from "./charts.module.css";

export interface CostPerAgentChartProps {
  query: TokenUsageQuery;
}

/**
 * Horizontal bar: blended-dollar cost per agent over the window. Backend
 * already returns agents sorted by `total_cost_usd` desc; we render in the
 * order received so the bar chart and the sibling scatter agree on
 * column order.
 */
export function CostPerAgentChart({ query }: CostPerAgentChartProps) {
  const result = useTokenUsageCostPerAgent(query);
  return (
    <ChartCard
      title="Cost per agent"
      testId="chart-cost-per-agent"
      isLoading={result.isLoading}
      error={result.error}
    >
      <CostPerAgentChartContent data={result.data} />
    </ChartCard>
  );
}

interface ContentProps {
  data: TokenUsageCostPerAgentResponse | undefined;
}

function CostPerAgentChartContent({ data }: ContentProps) {
  const rows = useMemo(
    () =>
      (data?.agents ?? []).map((a) => ({
        label: agentDisplayName(a.agent_name),
        total_cost_usd: a.total_cost_usd,
      })),
    [data],
  );

  if (rows.length === 0) {
    return <div className={styles.empty}>No data in this window</div>;
  }

  return (
    <div className={styles.chartContent} data-testid="cost-per-agent-content">
      <div className={styles.chartBody}>
        <ResponsiveContainer width="100%" height="100%">
          <BarChart
            data={rows}
            layout="vertical"
            margin={{ top: 8, right: 12, bottom: 0, left: 12 }}
          >
            <CartesianGrid strokeDasharray="3 3" stroke={GRID_STROKE} />
            <XAxis type="number" tick={AXIS_TICK} tickFormatter={formatUsd} />
            <YAxis
              type="category"
              dataKey="label"
              tick={AXIS_TICK}
              width={90}
              interval={0}
            />
            <Tooltip
              contentStyle={TOOLTIP_STYLE}
              formatter={(value) => [formatUsd(Number(value)), "Cost"]}
            />
            <Bar
              dataKey="total_cost_usd"
              fill={CHART_COLORS.accent}
              name="Cost"
              isAnimationActive={false}
            />
          </BarChart>
        </ResponsiveContainer>
      </div>
    </div>
  );
}
