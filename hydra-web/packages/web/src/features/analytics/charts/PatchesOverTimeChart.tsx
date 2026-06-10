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
import { CHART_COLORS } from "./colors";
import { formatBucketLabel } from "../duration";
import styles from "./charts.module.css";

export interface PatchesOverTimeChartProps {
  query: PatchesThroughputQuery;
}

/**
 * Stacked area: patches created vs merged per bucket. Stacked area picked
 * over grouped bars because the headline question is "are we shipping at
 * the rate we're taking work on" — the combined area conveys that lens
 * better than discrete bars at small card widths.
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
      (data?.buckets ?? []).map((b) => ({
        bucket_start: b.bucket_start,
        label: formatBucketLabel(b.bucket_start),
        created: Number(b.created),
        merged: Number(b.merged),
      })),
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
            <CartesianGrid strokeDasharray="3 3" stroke="#2a2a2a" />
            <XAxis dataKey="label" tick={{ fontSize: 11, fill: "#888" }} />
            <YAxis allowDecimals={false} tick={{ fontSize: 11, fill: "#888" }} />
            <Tooltip
              contentStyle={{ background: "#0e0e0e", border: "1px solid #2a2a2a" }}
            />
            <Area
              type="monotone"
              dataKey="created"
              stackId="1"
              stroke={CHART_COLORS.created}
              fill={CHART_COLORS.created}
              fillOpacity={0.5}
              name="Created"
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
          </AreaChart>
        </ResponsiveContainer>
      </div>
      <ul className={styles.legend}>
        <li className={styles.legendItem}>
          <span
            className={styles.legendSwatch}
            style={{ background: CHART_COLORS.created }}
          />
          Created
        </li>
        <li className={styles.legendItem}>
          <span
            className={styles.legendSwatch}
            style={{ background: CHART_COLORS.merged }}
          />
          Merged
        </li>
      </ul>
    </div>
  );
}
