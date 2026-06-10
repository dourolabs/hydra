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
import type { IssuesThroughputQuery, IssuesOverTimeResponse } from "@hydra/api";
import { ChartCard } from "../ChartCard";
import { useThroughputIssuesOverTime } from "../useThroughputIssues";
import { AXIS_TICK, CHART_COLORS, GRID_STROKE, TOOLTIP_STYLE } from "./colors";
import { formatBucketLabel } from "../duration";
import styles from "./charts.module.css";

export interface IssuesOverTimeChartProps {
  query: IssuesThroughputQuery;
}

/**
 * Stacked area: issues created vs reached-terminal per bucket. Mirrors
 * the patches `over_time` chart variant so the two sections of the page
 * read with the same visual idiom.
 */
export function IssuesOverTimeChart({ query }: IssuesOverTimeChartProps) {
  const result = useThroughputIssuesOverTime(query);
  return (
    <ChartCard
      title="Issues over time"
      testId="chart-issues-over-time"
      isLoading={result.isLoading}
      error={result.error}
    >
      <IssuesOverTimeChartContent data={result.data} />
    </ChartCard>
  );
}

function IssuesOverTimeChartContent({
  data,
}: {
  data: IssuesOverTimeResponse | undefined;
}) {
  const points = useMemo(
    () =>
      (data?.buckets ?? []).map((b) => ({
        bucket_start: b.bucket_start,
        label: formatBucketLabel(b.bucket_start),
        created: Number(b.created),
        reached_terminal: Number(b.reached_terminal),
      })),
    [data],
  );

  if (points.length === 0) {
    return <div className={styles.empty}>No data in this window</div>;
  }

  return (
    <div className={styles.chartContent} data-testid="issues-over-time-content">
      <div className={styles.chartBody}>
        <ResponsiveContainer width="100%" height="100%">
          <AreaChart data={points} margin={{ top: 8, right: 12, bottom: 0, left: 0 }}>
            <CartesianGrid strokeDasharray="3 3" stroke={GRID_STROKE} />
            <XAxis dataKey="label" tick={AXIS_TICK} />
            <YAxis allowDecimals={false} tick={AXIS_TICK} />
            <Tooltip contentStyle={TOOLTIP_STYLE} />
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
              dataKey="reached_terminal"
              stackId="1"
              stroke={CHART_COLORS.merged}
              fill={CHART_COLORS.merged}
              fillOpacity={0.7}
              name="Reached terminal"
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
          Reached terminal
        </li>
      </ul>
    </div>
  );
}
