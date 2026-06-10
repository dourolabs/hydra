import { useMemo } from "react";
import { Cell, Pie, PieChart, ResponsiveContainer, Tooltip } from "recharts";
import type {
  PatchesThroughputQuery,
  PatchesTerminalMixResponse,
} from "@hydra/api";
import { ChartCard } from "../ChartCard";
import { useThroughputPatchesTerminalMix } from "../useThroughputPatches";
import { CHART_COLORS, TOOLTIP_STYLE } from "./colors";
import styles from "./charts.module.css";

export interface PatchesTerminalMixChartProps {
  query: PatchesThroughputQuery;
}

/** Donut: merged vs closed share of terminal-status patches in the window. */
export function PatchesTerminalMixChart({ query }: PatchesTerminalMixChartProps) {
  const result = useThroughputPatchesTerminalMix(query);
  return (
    <ChartCard
      title="Terminal mix"
      testId="chart-patches-terminal-mix"
      isLoading={result.isLoading}
      error={result.error}
    >
      <PatchesTerminalMixChartContent data={result.data} />
    </ChartCard>
  );
}

function PatchesTerminalMixChartContent({
  data,
}: {
  data: PatchesTerminalMixResponse | undefined;
}) {
  const merged = Number(data?.merged ?? 0);
  const closed = Number(data?.closed ?? 0);
  const total = merged + closed;

  const slices = useMemo(
    () => [
      { name: "Merged", value: merged, color: CHART_COLORS.merged },
      { name: "Closed", value: closed, color: CHART_COLORS.closed },
    ],
    [merged, closed],
  );

  if (total === 0) {
    return <div className={styles.empty}>No data in this window</div>;
  }

  const mergedPct = Math.round((merged / total) * 100);
  const closedPct = 100 - mergedPct;

  return (
    <div className={styles.chartContent} data-testid="patches-terminal-mix-content">
      <div className={styles.donutWrapper}>
        <div className={styles.chartBody}>
          <ResponsiveContainer width="100%" height="100%">
            <PieChart>
              <Pie
                data={slices}
                dataKey="value"
                nameKey="name"
                innerRadius="55%"
                outerRadius="80%"
                paddingAngle={1}
                stroke="none"
                isAnimationActive={false}
              >
                {slices.map((s) => (
                  <Cell key={s.name} fill={s.color} />
                ))}
              </Pie>
              <Tooltip contentStyle={TOOLTIP_STYLE} />
            </PieChart>
          </ResponsiveContainer>
        </div>
        <div className={styles.donutCenter} aria-hidden="true">
          <span
            className={styles.donutCenterValue}
            data-testid="patches-terminal-mix-total"
          >
            {total}
          </span>
          <span className={styles.donutCenterLabel}>Total</span>
        </div>
      </div>
      <ul className={styles.legend}>
        <li className={styles.legendItem}>
          <span
            className={styles.legendSwatch}
            style={{ background: CHART_COLORS.merged }}
          />
          Merged: {merged} ({mergedPct}%)
        </li>
        <li className={styles.legendItem}>
          <span
            className={styles.legendSwatch}
            style={{ background: CHART_COLORS.closed }}
          />
          Closed: {closed} ({closedPct}%)
        </li>
      </ul>
    </div>
  );
}
