import { useMemo } from "react";
import type { IssueSummaryRecord, JobSummaryRecord } from "@metis/api";
import { buildIssueTree } from "../issues/useIssues";
import { treeHasActiveNode } from "./watchingUtils";
import { computeGlobalMetrics } from "./gridUtils";
import { WatchlistGridCard } from "./WatchlistGridCard";
import styles from "./WatchlistGrid.module.css";

interface WatchlistGridProps {
  issues: IssueSummaryRecord[];
  jobsByIssue: Map<string, JobSummaryRecord[]>;
  selectedId: string | null;
  onSelect: (issueId: string) => void;
  username: string;
}

export function WatchlistGrid({
  issues,
  jobsByIssue,
  selectedId,
  onSelect,
  username,
}: WatchlistGridProps) {
  const watchingRoots = useMemo(() => {
    const tree = buildIssueTree(issues);
    return tree
      .filter(
        (root) =>
          !root.hardBlocked &&
          root.issue.issue.creator === username &&
          treeHasActiveNode(root, jobsByIssue),
      )
      .sort(
        (a, b) =>
          new Date(b.issue.creation_time).getTime() -
          new Date(a.issue.creation_time).getTime(),
      );
  }, [issues, jobsByIssue, username]);

  const globalMetrics = useMemo(
    () => computeGlobalMetrics(watchingRoots, jobsByIssue),
    [watchingRoots, jobsByIssue],
  );

  if (watchingRoots.length === 0) {
    return <p className={styles.empty}>No active issues. Create an issue to get started.</p>;
  }

  return (
    <div className={styles.container}>
      {/* Global status bar */}
      <div className={styles.statusBar}>
        <span className={`${styles.statusCounter} ${styles.counterAgents}`}>
          {globalMetrics.agentsRunning > 0 && (
            <span className={styles.pulseDot} />
          )}
          {globalMetrics.agentsRunning} agents running
        </span>
        <span
          className={`${styles.statusCounter} ${styles.counterAttention} ${
            globalMetrics.needAttention > 0 ? styles.hasItems : ""
          }`}
        >
          ! {globalMetrics.needAttention} need attention
        </span>
        <span className={`${styles.statusCounter} ${styles.counterShipped}`}>
          ✓ {globalMetrics.shipped}/{globalMetrics.total} shipped
        </span>
      </div>

      {/* Issue grid */}
      <div className={styles.grid}>
        {watchingRoots.map((root) => (
          <WatchlistGridCard
            key={root.id}
            root={root}
            jobsByIssue={jobsByIssue}
            selectedId={selectedId}
            onSelect={onSelect}
          />
        ))}
      </div>
    </div>
  );
}
