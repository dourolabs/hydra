import { useCallback, useMemo } from "react";
import { useSearchParams } from "react-router-dom";
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

  const issuesOverTimeQuery = useMemo(
    () => ({ ...baseIssuesQuery, bucket: "day" as const }),
    [baseIssuesQuery],
  );
  const issuesProjectScopedQuery = useMemo(
    () => ({ ...baseIssuesQuery, project_id: state.projectId ?? "" }),
    [baseIssuesQuery, state.projectId],
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
              <PatchesOverTimeChart query={patchesOverTimeQuery} />
              <PatchesTerminalMixChart query={baseQuery} />
              <PatchesTimeToMergeChart query={baseQuery} />
              <PatchesInFlightChart query={patchesInFlightQuery} />
            </div>
          </section>

          <section
            data-testid="analytics-issues-section"
            className={styles.section}
            aria-label="Issues throughput"
          >
            <h2 className={styles.sectionTitle}>Issues</h2>
            <div className={styles.grid}>
              <IssuesOverTimeChart query={issuesOverTimeQuery} />
              <IssuesCycleTimeChart query={baseIssuesQuery} />
              <IssuesTimeInStatusBreakdownChart
                query={issuesProjectScopedQuery}
                hasProject={hasProject}
              />
              <IssuesPerStatusDistributionChart
                query={issuesProjectScopedQuery}
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
