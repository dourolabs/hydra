import type {
  IssuesPerStatusDistributionQuery,
  IssuesPerStatusDistributionResponse,
} from "@hydra/api";
import { ChartCard } from "../ChartCard";
import { useThroughputIssuesPerStatusDistribution } from "../useThroughputIssues";
import { formatDurationSeconds } from "../duration";
import styles from "./charts.module.css";

export interface IssuesPerStatusDistributionChartProps {
  query: IssuesPerStatusDistributionQuery;
  /** When false the card renders a "select a project" placeholder. */
  hasProject: boolean;
}

/**
 * Per-status percentile callouts: one card per status with median, p95
 * and sample count. Recharts has no first-class box-plot primitive at the
 * 2.x version we're on, so we render a card grid (per the parent issue's
 * "small table or set of cards" guidance) rather than approximating with
 * stacked bars.
 */
export function IssuesPerStatusDistributionChart({
  query,
  hasProject,
}: IssuesPerStatusDistributionChartProps) {
  const result = useThroughputIssuesPerStatusDistribution(query, hasProject);
  return (
    <ChartCard
      title="Per-status distribution"
      testId="chart-issues-per-status"
      disabled={!hasProject}
      disabledHint="Select a project to view this chart"
      isLoading={result.isLoading}
      error={result.error}
    >
      <IssuesPerStatusDistributionContent data={result.data} />
    </ChartCard>
  );
}

function IssuesPerStatusDistributionContent({
  data,
}: {
  data: IssuesPerStatusDistributionResponse | undefined;
}) {
  const statuses = data?.statuses ?? [];

  if (statuses.length === 0) {
    return <div className={styles.empty}>No data in this window</div>;
  }

  return (
    <div className={styles.chartContent} data-testid="issues-per-status-content">
      <ul className={styles.statusCardGrid}>
        {statuses.map((s) => (
          <li
            key={s.status_key}
            className={styles.statusCard}
            data-testid={`issues-per-status-card-${s.status_key}`}
          >
            <header className={styles.statusCardHead}>
              <span
                className={styles.legendSwatch}
                style={{ background: s.color }}
              />
              <span className={styles.statusCardLabel}>{s.label}</span>
            </header>
            <div className={styles.statusCardStat}>
              <span className={styles.statusCardStatLabel}>Median</span>
              <span className={styles.statusCardStatValue}>
                {s.median_seconds != null
                  ? formatDurationSeconds(s.median_seconds)
                  : "—"}
              </span>
            </div>
            <div className={styles.statusCardStat}>
              <span className={styles.statusCardStatLabel}>p95</span>
              <span className={styles.statusCardStatValue}>
                {s.p95_seconds != null
                  ? formatDurationSeconds(s.p95_seconds)
                  : "—"}
              </span>
            </div>
            <div className={styles.statusCardStat}>
              <span className={styles.statusCardStatLabel}>Samples</span>
              <span className={styles.statusCardStatValue}>
                {s.sample_count}
              </span>
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}
