import { useMemo } from "react";
import type {
  IssuesThroughputQuery,
  IssuesTimeInStatusBreakdownResponse,
  TimeInStatusSegment,
} from "@hydra/api";
import { ChartCard } from "../ChartCard";
import { useThroughputIssuesTimeInStatusBreakdown } from "../useThroughputIssues";
import { formatDurationSeconds } from "../../../utils/time";
import styles from "./charts.module.css";

export interface IssuesTimeInStatusBreakdownChartProps {
  query: IssuesThroughputQuery;
  /** When false the card renders a "select a project" placeholder. */
  hasProject: boolean;
}

/**
 * Horizontal stacked bar — one segment per project status, segment width
 * proportional to mean time-in-status across the cohort. Colors come
 * from the backend response (project's status definitions) and segments
 * render in the order the backend returned them (project priority).
 *
 * Picked a CSS-flex bar over recharts' vertical-layout BarChart because
 * this is a single-row stacked bar — flex with width percentages renders
 * sharper, scales to mobile widths, and keeps the segment hover labels
 * native (no recharts SVG layout in jsdom).
 */
export function IssuesTimeInStatusBreakdownChart({
  query,
  hasProject,
}: IssuesTimeInStatusBreakdownChartProps) {
  const result = useThroughputIssuesTimeInStatusBreakdown(query, hasProject);
  return (
    <ChartCard
      title="Time in status"
      testId="chart-issues-time-in-status"
      disabled={!hasProject}
      disabledHint="Select a project to view this chart"
      isLoading={result.isLoading}
      error={result.error}
    >
      <IssuesTimeInStatusBreakdownContent data={result.data} />
    </ChartCard>
  );
}

function IssuesTimeInStatusBreakdownContent({
  data,
}: {
  data: IssuesTimeInStatusBreakdownResponse | undefined;
}) {
  const segments = useMemo(
    () => data?.status_segments ?? [],
    [data?.status_segments],
  );
  const issueCount = Number(data?.issue_count ?? 0);

  const total = useMemo(
    () => segments.reduce((acc, s) => acc + Number(s.mean_seconds), 0),
    [segments],
  );

  if (segments.length === 0 || issueCount === 0) {
    return <div className={styles.empty}>No data in this window</div>;
  }

  return (
    <div className={styles.chartContent} data-testid="issues-time-in-status-content">
      <div
        className={styles.callouts}
        data-testid="issues-time-in-status-callouts"
      >
        <div className={styles.callout}>
          <span className={styles.calloutLabel}>Issues:</span>
          <span className={styles.calloutValue}>{issueCount}</span>
        </div>
        <div className={styles.callout}>
          <span className={styles.calloutLabel}>Mean total:</span>
          <span className={styles.calloutValue}>
            {formatDurationSeconds(total)}
          </span>
        </div>
      </div>
      <div
        className={styles.stackedBar}
        role="img"
        aria-label={`Mean time in status across ${segments.length} statuses`}
      >
        {segments.map((s) => (
          <StackedBarSegment key={s.status_key} segment={s} total={total} />
        ))}
      </div>
      <ul className={styles.legend}>
        {segments.map((s) => (
          <li
            key={s.status_key}
            className={styles.legendItem}
            tabIndex={0}
            title={`${s.label}: mean ${formatDurationSeconds(Number(s.mean_seconds))}`}
            data-testid={`issues-time-in-status-legend-${s.status_key}`}
          >
            <span
              className={styles.legendSwatch}
              style={{ background: s.color }}
            />
            <span>{s.label}</span>
            <span className={styles.legendValue}>
              {formatDurationSeconds(Number(s.mean_seconds))}
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}

function StackedBarSegment({
  segment,
  total,
}: {
  segment: TimeInStatusSegment;
  total: number;
}) {
  const meanSeconds = Number(segment.mean_seconds);
  const pct = total > 0 ? (meanSeconds / total) * 100 : 0;
  if (pct === 0) return null;
  return (
    <span
      className={styles.stackedBarSegment}
      style={{ width: `${pct}%`, background: segment.color }}
      title={`${segment.label}: ${formatDurationSeconds(meanSeconds)}`}
      data-testid={`issues-time-in-status-segment-${segment.status_key}`}
    />
  );
}
