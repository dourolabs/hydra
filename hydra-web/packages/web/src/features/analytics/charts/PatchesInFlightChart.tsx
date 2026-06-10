import { useMemo } from "react";
import {
  CartesianGrid,
  Line,
  LineChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import type {
  PatchesThroughputQuery,
  PatchesInFlightOverTimeResponse,
} from "@hydra/api";
import { ChartCard } from "../ChartCard";
import { useThroughputPatchesInFlightOverTime } from "../useThroughputPatches";
import { AXIS_TICK, CHART_COLORS, GRID_STROKE, TOOLTIP_STYLE } from "./colors";
import { formatBucketLabel } from "../duration";
import styles from "./charts.module.css";

export interface PatchesInFlightChartProps {
  query: PatchesThroughputQuery;
}

/** Line chart of `open + changes-requested` patch count at each bucket boundary. */
export function PatchesInFlightChart({ query }: PatchesInFlightChartProps) {
  const result = useThroughputPatchesInFlightOverTime(query);
  return (
    <ChartCard
      title="In-flight over time"
      testId="chart-patches-in-flight"
      isLoading={result.isLoading}
      error={result.error}
    >
      <PatchesInFlightChartContent data={result.data} />
    </ChartCard>
  );
}

function PatchesInFlightChartContent({
  data,
}: {
  data: PatchesInFlightOverTimeResponse | undefined;
}) {
  const points = useMemo(
    () =>
      (data?.buckets ?? []).map((b) => ({
        label: formatBucketLabel(b.bucket_start),
        in_flight: Number(b.in_flight),
      })),
    [data],
  );

  if (points.length === 0) {
    return <div className={styles.empty}>No data in this window</div>;
  }

  return (
    <div className={styles.chartContent} data-testid="patches-in-flight-content">
      <div className={styles.chartBody}>
        <ResponsiveContainer width="100%" height="100%">
          <LineChart data={points} margin={{ top: 8, right: 12, bottom: 0, left: 0 }}>
            <CartesianGrid strokeDasharray="3 3" stroke={GRID_STROKE} />
            <XAxis dataKey="label" tick={AXIS_TICK} />
            <YAxis allowDecimals={false} tick={AXIS_TICK} />
            <Tooltip contentStyle={TOOLTIP_STYLE} />
            <Line
              type="monotone"
              dataKey="in_flight"
              stroke={CHART_COLORS.accent}
              strokeWidth={2}
              dot={false}
              name="In flight"
            />
          </LineChart>
        </ResponsiveContainer>
      </div>
    </div>
  );
}
