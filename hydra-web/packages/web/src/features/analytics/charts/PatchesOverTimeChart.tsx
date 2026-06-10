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
import type { PatchesThroughputQuery, PatchesOverTimeResponse } from "@hydra/api";
import { ChartCard } from "../ChartCard";
import { useThroughputPatchesOverTime } from "../useThroughputPatches";
import { AXIS_TICK, CHART_COLORS, GRID_STROKE, TOOLTIP_STYLE } from "./colors";
import { formatBucketLabel } from "../duration";
import styles from "./charts.module.css";

export interface PatchesOverTimeChartProps {
  query: PatchesThroughputQuery;
}

/**
 * Stacked area: merged on the bottom, delta = created − merged on top.
 * The stack height represents the total created in each bucket, so the
 * green portion shows what shipped and the gray fills up to the total.
 */
export function PatchesOverTimeChart({ query }: PatchesOverTimeChartProps) {
  const result = useThroughputPatchesOverTime(query);
  return (
    <ChartCard
      title="Patches over time"
      testId="chart-patches-over-time"
      isLoading={result.isLoading}
      error={result.error}
    >
      <PatchesOverTimeChartContent data={result.data} />
    </ChartCard>
  );
}

interface ContentProps {
  data: PatchesOverTimeResponse | undefined;
}

function PatchesOverTimeChartContent({ data }: ContentProps) {
  const points = useMemo(
    () =>
      (data?.buckets ?? []).map((b) => {
        const total = Number(b.created);
        const merged = Number(b.merged);
        return {
          bucket_start: b.bucket_start,
          label: formatBucketLabel(b.bucket_start),
          merged,
          // Clamped: a bucket whose merged exceeds created (patches merged
          // in this bucket but created earlier) would otherwise yield a
          // negative slice.
          delta: Math.max(0, total - merged),
          total,
        };
      }),
    [data],
  );

  if (points.length === 0) {
    return <div className={styles.empty}>No data in this window</div>;
  }

  return (
    <div className={styles.chartContent} data-testid="patches-over-time-content">
      <div className={styles.chartBody}>
        <ResponsiveContainer width="100%" height="100%">
          <AreaChart data={points} margin={{ top: 8, right: 12, bottom: 0, left: 0 }}>
            <CartesianGrid strokeDasharray="3 3" stroke={GRID_STROKE} />
            <XAxis dataKey="label" tick={AXIS_TICK} />
            <YAxis allowDecimals={false} tick={AXIS_TICK} />
            <Tooltip
              contentStyle={TOOLTIP_STYLE}
              formatter={(value, name, item) => {
                // The "Total" series stacks the delta; surface the actual
                // total (which is the stack's upper boundary) in the tooltip
                // so the displayed number matches the legend label.
                if (name === "Total") {
                  const point = item.payload as { total: number };
                  return [point.total, name];
                }
                return [value as number, name];
              }}
            />
            <Area
              type="monotone"
              dataKey="merged"
              stackId="1"
              stroke={CHART_COLORS.merged}
              fill={CHART_COLORS.merged}
              fillOpacity={0.7}
              name="Merged"
            />
            <Area
              type="monotone"
              dataKey="delta"
              stackId="1"
              stroke={CHART_COLORS.created}
              fill={CHART_COLORS.created}
              fillOpacity={0.5}
              name="Total"
            />
          </AreaChart>
        </ResponsiveContainer>
      </div>
      <ul className={styles.legend}>
        <li className={styles.legendItem}>
          <span
            className={styles.legendSwatch}
            style={{ "--swatch": CHART_COLORS.merged } as React.CSSProperties}
          />
          Merged
        </li>
        <li className={styles.legendItem}>
          <span
            className={styles.legendSwatch}
            style={{ "--swatch": CHART_COLORS.created } as React.CSSProperties}
          />
          Total
        </li>
      </ul>
    </div>
  );
}
