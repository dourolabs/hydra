import { useCallback, useMemo } from "react";
import { useSearchParams } from "react-router-dom";
import type { TokenUsageOverTimeQuery, TokenUsageQuery } from "@hydra/api";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import { PageHead } from "../layout/PageHead";
import { TimeRangePicker } from "../features/analytics/TimeRangePicker";
import {
  DEFAULT_TIME_RANGE,
  isTimeRange,
  timeWindow,
  type TimeRange,
} from "../features/analytics/slicerState";
import {
  CostOverTimeChart,
  CostPerAgentChart,
  PerSessionCostScatterChart,
  TokensOverTimeChart,
  TopIssuesByCostList,
} from "../features/analytics/charts";
import styles from "./AnalyticsTokenUsagePage.module.css";

const RANGE_PARAM = "range";

const RANGE_EYEBROW: Record<TimeRange, string> = {
  "7d": "Last 7 days",
  "30d": "Last 30 days",
  "90d": "Last 90 days",
  "all-time": "All time",
};

export function AnalyticsTokenUsagePage() {
  useBreadcrumbs([{ label: "Analytics", to: "/analytics" }], "Token Usage");

  const [searchParams, setSearchParams] = useSearchParams();
  const range = useMemo<TimeRange>(() => {
    const raw = searchParams.get(RANGE_PARAM);
    return raw && isTimeRange(raw) ? raw : DEFAULT_TIME_RANGE;
  }, [searchParams]);

  const onRangeChange = useCallback(
    (next: TimeRange) => {
      setSearchParams((prev) => {
        const params = new URLSearchParams(prev);
        params.set(RANGE_PARAM, next);
        return params;
      });
    },
    [setSearchParams],
  );

  const window = useMemo(() => timeWindow(range), [range]);

  const overTimeQuery = useMemo<TokenUsageOverTimeQuery>(
    () => ({
      from: window.from,
      to: window.to,
      bucket: "day",
      repo_name: null,
      creator: null,
    }),
    [window],
  );

  const costQuery = useMemo<TokenUsageQuery>(
    () => ({
      from: window.from,
      to: window.to,
      repo_name: null,
      creator: null,
    }),
    [window],
  );

  return (
    <div className={styles.page} data-testid="analytics-token-usage-page">
      <PageHead
        eyebrow={RANGE_EYEBROW[range]}
        title="Token Usage"
        actions={<TimeRangePicker value={range} onChange={onRangeChange} />}
      />

      <div className={styles.body}>
        <section
          data-testid="analytics-tokens-section"
          className={styles.section}
          aria-label="Token usage over time"
        >
          <h2 className={styles.sectionTitle}>Over time</h2>
          <div className={styles.grid}>
            <TokensOverTimeChart query={overTimeQuery} />
            <CostOverTimeChart query={overTimeQuery} />
          </div>
        </section>

        <section
          data-testid="analytics-by-agent-section"
          className={styles.section}
          aria-label="Cost by agent"
        >
          <h2 className={styles.sectionTitle}>By agent</h2>
          <div className={styles.grid}>
            <CostPerAgentChart query={costQuery} />
            <PerSessionCostScatterChart query={costQuery} />
          </div>
        </section>

        <section
          data-testid="analytics-top-issues-section"
          className={styles.section}
          aria-label="Top 10 most expensive issues"
        >
          <h2 className={styles.sectionTitle}>Top issues</h2>
          <TopIssuesByCostList query={costQuery} />
        </section>
      </div>
    </div>
  );
}
