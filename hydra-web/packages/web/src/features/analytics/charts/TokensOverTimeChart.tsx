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
import styles from "./charts.module.css";

export interface TokensOverTimeChartProps {
  query: TokenUsageOverTimeQuery;
}

const SERIES = [
  { key: "input_tokens", label: "Input", color: CHART_COLORS.tokensInput },
  { key: "output_tokens", label: "Output", color: CHART_COLORS.tokensOutput },
  {
    key: "cache_read_input_tokens",
    label: "Cache read",
    color: CHART_COLORS.tokensCacheRead,
  },
  {
    key: "cache_creation_input_tokens",
    label: "Cache write",
    color: CHART_COLORS.tokensCacheWrite,
  },
] as const;

/**
 * Stacked area: token counts per bucket, split across the four token
 * categories the backend returns. Stacked because the headline lens is
 * "where is our spend going" — totals + per-bucket share read together.
 */
export function TokensOverTimeChart({ query }: TokensOverTimeChartProps) {
  const result = useTokenUsageOverTime(query);
  return (
    <ChartCard
      title="Tokens over time"
      testId="chart-tokens-over-time"
      isLoading={result.isLoading}
      error={result.error}
    >
      <TokensOverTimeChartContent data={result.data} />
    </ChartCard>
  );
}

interface ContentProps {
  data: TokenUsageOverTimeResponse | undefined;
}

function TokensOverTimeChartContent({ data }: ContentProps) {
  const points = useMemo(
    () =>
      (data?.buckets ?? []).map((b) => ({
        bucket_start: b.bucket_start,
        label: formatBucketLabel(b.bucket_start),
        input_tokens: Number(b.input_tokens),
        output_tokens: Number(b.output_tokens),
        cache_read_input_tokens: Number(b.cache_read_input_tokens),
        cache_creation_input_tokens: Number(b.cache_creation_input_tokens),
      })),
    [data],
  );

  if (points.length === 0) {
    return <div className={styles.empty}>No data in this window</div>;
  }

  return (
    <div className={styles.chartContent} data-testid="tokens-over-time-content">
      <div className={styles.chartBody}>
        <ResponsiveContainer width="100%" height="100%">
          <AreaChart data={points} margin={{ top: 8, right: 12, bottom: 0, left: 0 }}>
            <CartesianGrid strokeDasharray="3 3" stroke={GRID_STROKE} />
            <XAxis dataKey="label" tick={AXIS_TICK} />
            <YAxis allowDecimals={false} tick={AXIS_TICK} />
            <Tooltip contentStyle={TOOLTIP_STYLE} />
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
            data-testid={`tokens-over-time-legend-${s.key}`}
          >
            <span className={styles.legendSwatch} style={{ background: s.color }} />
            {s.label}
          </li>
        ))}
      </ul>
    </div>
  );
}
