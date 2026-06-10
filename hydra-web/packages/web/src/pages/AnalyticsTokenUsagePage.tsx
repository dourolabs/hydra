import { useCallback, useMemo } from "react";
import { useSearchParams } from "react-router-dom";
import type { TokenUsageOverTimeQuery, TokenUsageQuery } from "@hydra/api";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import { TimeRangePicker } from "../features/analytics/TimeRangePicker";
import {
  DEFAULT_TIME_RANGE,
  isTimeRange,
  timeWindow,
  type TimeRange,
} from "../features/analytics/slicerState";
import {
  CostPerAgentChart,
  PerSessionCostScatterChart,
  TokensOverTimeChart,
  TopIssuesByCostList,
} from "../features/analytics/charts";
import styles from "./AnalyticsTokenUsagePage.module.css";

const RANGE_PARAM = "range";

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
      <header className={styles.head}>
        <h1 className={styles.title}>Token Usage</h1>
        <TimeRangePicker value={range} onChange={onRangeChange} />
      </header>

      <div className={styles.body}>
        <section
          data-testid="analytics-tokens-section"
          className={styles.section}
          aria-label="Token usage over time"
        >
          <div className={styles.grid}>
            <TokensOverTimeChart query={overTimeQuery} />
          </div>
        </section>

        <section
          data-testid="analytics-cost-section"
          className={styles.section}
          aria-label="Token usage cost breakdown"
        >
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
          <TopIssuesByCostList query={costQuery} />
        </section>
      </div>
    </div>
  );
}
