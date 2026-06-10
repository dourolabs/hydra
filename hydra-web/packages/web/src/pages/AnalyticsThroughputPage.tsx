import { useCallback, useMemo } from "react";
import { useSearchParams } from "react-router-dom";
import type { IssuesThroughputQuery, PatchesThroughputQuery } from "@hydra/api";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import { SlicerPanel } from "../features/analytics/SlicerPanel";
import { TimeRangePicker } from "../features/analytics/TimeRangePicker";
import {
  readSlicerState,
  writeSlicerState,
  timeWindow,
  type SlicerState,
} from "../features/analytics/slicerState";
import {
  PatchesOverTimeChart,
  PatchesTerminalMixChart,
  PatchesTimeToMergeChart,
  PatchesInFlightChart,
  IssuesOverTimeChart,
  IssuesCycleTimeChart,
  IssuesTimeInStatusBreakdownChart,
  IssuesPerStatusDistributionChart,
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

  const patchesQuery = useMemo<PatchesThroughputQuery>(
    () => ({
      from: window.from,
      to: window.to,
      bucket: "day",
      repo_name: state.repoName,
      creator: state.creator,
    }),
    [window, state.repoName, state.creator],
  );

  const issuesQuery = useMemo<IssuesThroughputQuery>(
    () => ({
      from: window.from,
      to: window.to,
      bucket: "day",
      project_id: state.projectId,
      repo_name: state.repoName,
      issue_type: null,
      assignee: state.assignee,
      creator: state.creator,
      ...(state.issueTypes.length > 0
        ? { issue_types: state.issueTypes.join(",") }
        : {}),
      ...(state.statusKeys.length > 0
        ? { status_keys: state.statusKeys.join(",") }
        : {}),
    }),
    [
      window,
      state.projectId,
      state.repoName,
      state.issueTypes,
      state.assignee,
      state.creator,
      state.statusKeys,
    ],
  );

  const hasProject = !!state.projectId;

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
            aria-label="Patches throughput"
          >
            <h2 className={styles.sectionTitle}>Patches</h2>
            <div className={styles.grid}>
              <PatchesOverTimeChart query={patchesQuery} />
              <PatchesTerminalMixChart query={patchesQuery} />
              <PatchesTimeToMergeChart query={patchesQuery} />
              <PatchesInFlightChart query={patchesQuery} />
            </div>
          </section>

          <section
            data-testid="analytics-issues-section"
            className={styles.section}
            aria-label="Issues throughput"
          >
            <h2 className={styles.sectionTitle}>Issues</h2>
            <div className={styles.grid}>
              <IssuesOverTimeChart query={issuesQuery} />
              <IssuesCycleTimeChart query={issuesQuery} />
              <IssuesTimeInStatusBreakdownChart
                query={issuesQuery}
                hasProject={hasProject}
              />
              <IssuesPerStatusDistributionChart
                query={issuesQuery}
                hasProject={hasProject}
              />
            </div>
          </section>
        </div>

        <SlicerPanel state={state} onChange={onSlicerChange} />
      </div>
    </div>
  );
}
