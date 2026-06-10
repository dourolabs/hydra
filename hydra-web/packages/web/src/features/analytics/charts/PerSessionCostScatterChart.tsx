import { useMemo } from "react";
import {
  CartesianGrid,
  ResponsiveContainer,
  Scatter,
  ScatterChart,
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
import { agentDisplayName, formatUsd, sessionJitter } from "./cost";
import { AXIS_TICK, CHART_COLORS, GRID_STROKE } from "./colors";
import styles from "./charts.module.css";

export interface PerSessionCostScatterChartProps {
  query: TokenUsageQuery;
}

/**
 * Scatter: one point per session, grouped into rows by agent. Agent row
 * order matches the bar chart so the two read together (labels on Y,
 * cost in USD on X). Y position within a row is a deterministic jitter
 * on `session_id` to keep points stable across renders and Playwright
 * counts.
 */
export function PerSessionCostScatterChart({ query }: PerSessionCostScatterChartProps) {
  const result = useTokenUsageCostPerAgent(query);
  return (
    <ChartCard
      title="Per-session cost distribution"
      testId="chart-per-session-cost"
      isLoading={result.isLoading}
      error={result.error}
    >
      <PerSessionCostScatterChartContent data={result.data} />
    </ChartCard>
  );
}

interface ContentProps {
  data: TokenUsageCostPerAgentResponse | undefined;
}

interface ScatterPoint {
  y: number;
  cost_usd: number;
  session_id: string;
  agent_label: string;
}

function PerSessionCostScatterChartContent({ data }: ContentProps) {
  const { points, labels, ticks, domain } = useMemo(() => {
    const agents = data?.agents ?? [];
    const labels = agents.map((a) => agentDisplayName(a.agent_name));
    const points: ScatterPoint[] = [];
    agents.forEach((agent, agentIdx) => {
      for (const s of agent.sessions) {
        points.push({
          y: agentIdx + sessionJitter(s.session_id),
          cost_usd: s.cost_usd,
          session_id: s.session_id,
          agent_label: labels[agentIdx],
        });
      }
    });
    const ticks = agents.map((_, i) => i);
    const domain: [number, number] = [-0.5, Math.max(agents.length - 0.5, 0.5)];
    return { points, labels, ticks, domain };
  }, [data]);

  if (labels.length === 0) {
    return <div className={styles.empty}>No data in this window</div>;
  }

  return (
    <div className={styles.chartContent} data-testid="per-session-cost-content">
      <div className={styles.chartBody}>
        <ResponsiveContainer width="100%" height="100%">
          <ScatterChart margin={{ top: 8, right: 12, bottom: 0, left: 12 }}>
            <CartesianGrid strokeDasharray="3 3" stroke={GRID_STROKE} />
            <XAxis
              type="number"
              dataKey="cost_usd"
              tick={AXIS_TICK}
              tickFormatter={formatUsd}
            />
            <YAxis
              type="number"
              dataKey="y"
              domain={domain}
              ticks={ticks}
              tick={AXIS_TICK}
              tickFormatter={(v: number) => labels[v] ?? ""}
              interval={0}
              width={90}
              reversed
            />
            <Tooltip
              cursor={{ strokeDasharray: "3 3" }}
              content={<ScatterTooltip />}
            />
            <Scatter
              data={points}
              fill={CHART_COLORS.accent}
              isAnimationActive={false}
            />
          </ScatterChart>
        </ResponsiveContainer>
      </div>
    </div>
  );
}

interface TooltipPayload {
  payload: ScatterPoint;
}

function ScatterTooltip({
  active,
  payload,
}: {
  active?: boolean;
  payload?: TooltipPayload[];
}) {
  if (!active || !payload || payload.length === 0) return null;
  const p = payload[0].payload;
  return (
    <div className={styles.scatterTooltip}>
      <div className={styles.scatterTooltipId}>{p.session_id}</div>
      <div className={styles.scatterTooltipValue}>{formatUsd(p.cost_usd)}</div>
    </div>
  );
}
