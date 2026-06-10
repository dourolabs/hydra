import { useCallback, useMemo } from "react";
import { useSearchParams } from "react-router-dom";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import { SlicerPanel } from "../features/analytics/SlicerPanel";
import { TimeRangePicker } from "../features/analytics/TimeRangePicker";
import { ChartCard } from "../features/analytics/ChartCard";
import {
  readSlicerState,
  writeSlicerState,
  timeWindow,
  type SlicerState,
} from "../features/analytics/slicerState";
import {
  useThroughputIssuesCycleTime,
  useThroughputIssuesTimeInStatusBreakdown,
  useThroughputIssuesPerStatusDistribution,
  useThroughputIssuesOverTime,
} from "../features/analytics/useThroughputIssues";
import {
  PatchesOverTimeChart,
  PatchesTerminalMixChart,
  PatchesTimeToMergeChart,
  PatchesInFlightChart,
} from "../features/analytics/charts";
import styles from "./AnalyticsThroughputPage.module.css";

export function AnalyticsThroughputPage() {
  useBreadcrumbs([{ label: "Analytics", to: "/analytics" }], "Throughput");

  const [searchParams, setSearchParams] = useSearchParams();
  const state = useMemo<SlicerState>(() => readSlicerState(searchParams), [searchParams]);

  const onSlicerChange = useCallback(
    (patch: Partial<SlicerState>) => {
      setSearchParams((prev) => writeSlicerState(prev, patch));
    },
    [setSearchParams],
  );

  const window = useMemo(() => timeWindow(state.range), [state.range]);

  const baseQuery = useMemo(
    () => ({
      from: window.from,
      to: window.to,
      ...(state.repoName ? { repo_name: state.repoName } : {}),
      ...(state.creator ? { creator: state.creator } : {}),
    }),
    [window, state.repoName, state.creator],
  );

  const baseIssuesQuery = useMemo(
    () => ({
      ...baseQuery,
      ...(state.projectId ? { project_id: state.projectId } : {}),
      ...(state.issueTypes.length > 0 ? { issue_types: state.issueTypes.join(",") } : {}),
      ...(state.assignee ? { assignee: state.assignee } : {}),
      ...(state.statusKeys.length > 0 ? { status_keys: state.statusKeys.join(",") } : {}),
    }),
    [baseQuery, state.projectId, state.issueTypes, state.assignee, state.statusKeys],
  );

  const patchesOverTimeQuery = useMemo(
    () => ({ ...baseQuery, bucket: "day" as const }),
    [baseQuery],
  );
  const patchesInFlightQuery = useMemo(
    () => ({ ...baseQuery, bucket: "day" as const }),
    [baseQuery],
  );

  const issuesOverTime = useThroughputIssuesOverTime({ ...baseIssuesQuery, bucket: "day" });
  const issuesCycleTime = useThroughputIssuesCycleTime(baseIssuesQuery);
  const issuesTimeInStatus = useThroughputIssuesTimeInStatusBreakdown({
    ...baseIssuesQuery,
    project_id: state.projectId ?? "",
  });
  const issuesPerStatus = useThroughputIssuesPerStatusDistribution({
    ...baseIssuesQuery,
    project_id: state.projectId ?? "",
  });

  return (
    <div className={styles.page} data-testid="analytics-throughput-page">
      <header className={styles.head}>
        <div className={styles.headLeft}>
          <span className={styles.eyebrow}>ANALYTICS</span>
          <h1 className={styles.title}>Throughput</h1>
        </div>
        <TimeRangePicker
          value={state.range}
          onChange={(range) => onSlicerChange({ range })}
        />
      </header>

      <div className={styles.body}>
        <div className={styles.main}>
          <section
            data-testid="analytics-patches-section"
            className={styles.section}
          >
            <h2 className={styles.sectionTitle}>Patches</h2>
            <div className={styles.grid}>
              <PatchesOverTimeChart query={patchesOverTimeQuery} />
              <PatchesTerminalMixChart query={baseQuery} />
              <PatchesTimeToMergeChart query={baseQuery} />
              <PatchesInFlightChart query={patchesInFlightQuery} />
            </div>
          </section>

          <section
            data-testid="analytics-issues-section"
            className={styles.section}
          >
            <h2 className={styles.sectionTitle}>Issues</h2>
            <div className={styles.grid}>
              <ChartCard
                title="Issues over time"
                testId="chart-issues-over-time"
                isLoading={issuesOverTime.isLoading}
                error={issuesOverTime.error}
              >
                Chart coming soon
              </ChartCard>
              <ChartCard
                title="Cycle time"
                testId="chart-issues-cycle-time"
                isLoading={issuesCycleTime.isLoading}
                error={issuesCycleTime.error}
              >
                Chart coming soon
              </ChartCard>
              <ChartCard
                title="Time in status"
                testId="chart-issues-time-in-status"
                disabled={!state.projectId}
                disabledHint="Select a project"
                isLoading={issuesTimeInStatus.isLoading}
                error={issuesTimeInStatus.error}
              >
                Chart coming soon
              </ChartCard>
              <ChartCard
                title="Per-status distribution"
                testId="chart-issues-per-status"
                disabled={!state.projectId}
                disabledHint="Select a project"
                isLoading={issuesPerStatus.isLoading}
                error={issuesPerStatus.error}
              >
                Chart coming soon
              </ChartCard>
            </div>
          </section>
        </div>

        <SlicerPanel state={state} onChange={onSlicerChange} />
      </div>
    </div>
  );
}
