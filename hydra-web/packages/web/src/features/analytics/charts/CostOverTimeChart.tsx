import { useMemo } from "react";
import {
  Area,
  AreaChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import type { TokenUsageOverTimeQuery, TokenUsageOverTimeResponse } from "@hydra/api";
import { ChartCard } from "../ChartCard";
import { useTokenUsageOverTime } from "../useTokenUsage";
import { AXIS_TICK, CHART_COLORS, GRID_STROKE, TOOLTIP_STYLE } from "./colors";
import { formatBucketLabel } from "../duration";
import { formatUsd } from "./cost";
import { tokenCostUsd } from "./pricing";
import styles from "./charts.module.css";

export interface CostOverTimeChartProps {
  query: TokenUsageOverTimeQuery;
}

const SERIES = [
  {
    key: "input_cost_usd",
    countKey: "input_tokens",
    label: "Input",
    color: CHART_COLORS.tokensInput,
  },
  {
    key: "output_cost_usd",
    countKey: "output_tokens",
    label: "Output",
    color: CHART_COLORS.tokensOutput,
  },
  {
    key: "cache_read_cost_usd",
    countKey: "cache_read_input_tokens",
    label: "Cache read",
    color: CHART_COLORS.tokensCacheRead,
  },
  {
    key: "cache_write_cost_usd",
    countKey: "cache_creation_input_tokens",
    label: "Cache write",
    color: CHART_COLORS.tokensCacheWrite,
  },
] as const;

/**
 * Stacked area: per-bucket USD cost, broken down by token kind. Companion
 * to {@link TokensOverTimeChart} — same shape, same series colors, same
 * legend, but Y axis is dollars (counts × per-token rate from `pricing.ts`)
 * instead of raw tokens.
 */
export function CostOverTimeChart({ query }: CostOverTimeChartProps) {
  const result = useTokenUsageOverTime(query);
  return (
    <ChartCard
      title="Cost over time"
      testId="chart-cost-over-time"
      isLoading={result.isLoading}
      error={result.error}
    >
      <CostOverTimeChartContent data={result.data} />
    </ChartCard>
  );
}

interface ContentProps {
  data: TokenUsageOverTimeResponse | undefined;
}

function CostOverTimeChartContent({ data }: ContentProps) {
  const points = useMemo(
    () =>
      (data?.buckets ?? []).map((b) => ({
        bucket_start: b.bucket_start,
        label: formatBucketLabel(b.bucket_start),
        input_cost_usd: tokenCostUsd("input_tokens", Number(b.input_tokens)),
        output_cost_usd: tokenCostUsd("output_tokens", Number(b.output_tokens)),
        cache_read_cost_usd: tokenCostUsd(
          "cache_read_input_tokens",
          Number(b.cache_read_input_tokens),
        ),
        cache_write_cost_usd: tokenCostUsd(
          "cache_creation_input_tokens",
          Number(b.cache_creation_input_tokens),
        ),
      })),
    [data],
  );

  if (points.length === 0) {
    return <div className={styles.empty}>No data in this window</div>;
  }

  return (
    <div className={styles.chartContent} data-testid="cost-over-time-content">
      <div className={styles.chartBody}>
        <ResponsiveContainer width="100%" height="100%">
          <AreaChart data={points} margin={{ top: 8, right: 12, bottom: 0, left: 0 }}>
            <CartesianGrid strokeDasharray="3 3" stroke={GRID_STROKE} />
            <XAxis dataKey="label" tick={AXIS_TICK} />
            <YAxis tick={AXIS_TICK} tickFormatter={formatUsd} />
            <Tooltip
              contentStyle={TOOLTIP_STYLE}
              formatter={(value, name) => [formatUsd(Number(value)), name]}
            />
            {SERIES.map((s) => (
              <Area
                key={s.key}
                type="monotone"
                dataKey={s.key}
                stackId="1"
                stroke={s.color}
                fill={s.color}
                fillOpacity={0.6}
                name={s.label}
              />
            ))}
          </AreaChart>
        </ResponsiveContainer>
      </div>
      <ul className={styles.legend}>
        {SERIES.map((s) => (
          <li
            key={s.key}
            className={styles.legendItem}
            data-testid={`cost-over-time-legend-${s.countKey}`}
          >
            <span className={styles.legendSwatch} style={{ background: s.color }} />
            {s.label}
          </li>
        ))}
      </ul>
    </div>
  );
}
