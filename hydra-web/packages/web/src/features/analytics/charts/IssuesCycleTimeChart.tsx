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
  IssuesThroughputQuery,
  IssuesCycleTimeResponse,
} from "@hydra/api";
import { ChartCard } from "../ChartCard";
import { useThroughputIssuesCycleTime } from "../useThroughputIssues";
import { CHART_COLORS } from "./colors";
import { formatBinRange, formatDurationSeconds } from "../duration";
import styles from "./charts.module.css";

export interface IssuesCycleTimeChartProps {
  query: IssuesThroughputQuery;
}

/**
 * Histogram of issue created→terminal duration, with median + p95
 * callouts. Mirrors the patches `time_to_merge` chart so the analytics
 * page reads consistently across both sections.
 */
export function IssuesCycleTimeChart({ query }: IssuesCycleTimeChartProps) {
  const result = useThroughputIssuesCycleTime(query);
  return (
    <ChartCard
      title="Cycle time"
      testId="chart-issues-cycle-time"
      isLoading={result.isLoading}
      error={result.error}
    >
      <IssuesCycleTimeChartContent data={result.data} />
    </ChartCard>
  );
}

function IssuesCycleTimeChartContent({
  data,
}: {
  data: IssuesCycleTimeResponse | undefined;
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
    <div className={styles.chartContent} data-testid="issues-cycle-time-content">
      <div
        className={styles.callouts}
        data-testid="issues-cycle-time-callouts"
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
            <CartesianGrid strokeDasharray="3 3" stroke="#2a2a2a" />
            <XAxis
              dataKey="label"
              tick={{ fontSize: 11, fill: "#888" }}
              interval={0}
            />
            <YAxis allowDecimals={false} tick={{ fontSize: 11, fill: "#888" }} />
            <Tooltip
              contentStyle={{ background: "#0e0e0e", border: "1px solid #2a2a2a" }}
            />
            <Bar dataKey="count" fill={CHART_COLORS.accent} name="Issues" />
          </BarChart>
        </ResponsiveContainer>
      </div>
    </div>
  );
}
