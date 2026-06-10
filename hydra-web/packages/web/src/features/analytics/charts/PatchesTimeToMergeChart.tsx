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
  PatchesThroughputQuery,
  PatchesTimeToMergeResponse,
} from "@hydra/api";
import { ChartCard } from "../ChartCard";
import { useThroughputPatchesTimeToMerge } from "../useThroughputPatches";
import { AXIS_TICK, CHART_COLORS, GRID_STROKE, TOOLTIP_STYLE } from "./colors";
import { formatBinRange } from "../duration";
import { formatDurationSeconds } from "../../../utils/time";
import styles from "./charts.module.css";

export interface PatchesTimeToMergeChartProps {
  query: PatchesThroughputQuery;
}

/** Histogram of created→merged duration, with median + p95 callouts. */
export function PatchesTimeToMergeChart({ query }: PatchesTimeToMergeChartProps) {
  const result = useThroughputPatchesTimeToMerge(query);
  return (
    <ChartCard
      title="Time to merge"
      testId="chart-patches-time-to-merge"
      isLoading={result.isLoading}
      error={result.error}
    >
      <PatchesTimeToMergeChartContent data={result.data} />
    </ChartCard>
  );
}

function PatchesTimeToMergeChartContent({
  data,
}: {
  data: PatchesTimeToMergeResponse | undefined;
}) {
  const bins = useMemo(
    () =>
      (data?.histogram ?? []).map((b) => ({
        label: formatBinRange(
          Number(b.bin_start_seconds),
          b.bin_end_seconds == null ? null : Number(b.bin_end_seconds),
        ),
        count: Number(b.count),
      })),
    [data],
  );

  const count = Number(data?.count ?? 0);
  if (count === 0 || bins.length === 0) {
    return <div className={styles.empty}>No data in this window</div>;
  }

  return (
    <div className={styles.chartContent} data-testid="patches-time-to-merge-content">
      <div
        className={styles.callouts}
        data-testid="patches-time-to-merge-callouts"
      >
        <div className={styles.callout}>
          <span className={styles.calloutLabel}>Median:</span>
          <span className={styles.calloutValue}>
            {data?.median_seconds != null
              ? formatDurationSeconds(Number(data.median_seconds))
              : "—"}
          </span>
        </div>
        <div className={styles.callout}>
          <span className={styles.calloutLabel}>p95:</span>
          <span className={styles.calloutValue}>
            {data?.p95_seconds != null
              ? formatDurationSeconds(Number(data.p95_seconds))
              : "—"}
          </span>
        </div>
        <div className={styles.callout}>
          <span className={styles.calloutLabel}>Count:</span>
          <span className={styles.calloutValue}>{count}</span>
        </div>
      </div>
      <div className={styles.chartBody}>
        <ResponsiveContainer width="100%" height="100%">
          <BarChart data={bins} margin={{ top: 8, right: 12, bottom: 0, left: 0 }}>
            <CartesianGrid strokeDasharray="3 3" stroke={GRID_STROKE} />
            <XAxis dataKey="label" tick={AXIS_TICK} interval={0} />
            <YAxis allowDecimals={false} tick={AXIS_TICK} />
            <Tooltip contentStyle={TOOLTIP_STYLE} />
            <Bar dataKey="count" fill={CHART_COLORS.accent} name="Patches" />
          </BarChart>
        </ResponsiveContainer>
      </div>
    </div>
  );
}
