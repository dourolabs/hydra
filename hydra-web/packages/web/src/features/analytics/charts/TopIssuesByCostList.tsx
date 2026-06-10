import { Link } from "react-router-dom";
import type {
  TokenUsageQuery,
  TokenUsageTopIssuesByCostResponse,
} from "@hydra/api";
import { ChartCard } from "../ChartCard";
import { useTokenUsageTopIssuesByCost } from "../useTokenUsage";
import { formatUsd } from "./cost";
import styles from "./charts.module.css";

export interface TopIssuesByCostListProps {
  query: TokenUsageQuery;
}

/**
 * Top-cost issues list (rank + title + cost + session count). Backend
 * truncates to 10 and sorts by `cost_usd` desc; we render in the order
 * received. Lives in `charts/` next to the recharts widgets so analytics
 * surface code stays colocated, matching the precedent set by
 * `IssuesPerStatusDistributionChart` (also list-shaped).
 */
export function TopIssuesByCostList({ query }: TopIssuesByCostListProps) {
  const result = useTokenUsageTopIssuesByCost(query);
  return (
    <ChartCard
      title="Top 10 most expensive issues"
      testId="chart-top-issues-by-cost"
      isLoading={result.isLoading}
      error={result.error}
    >
      <TopIssuesByCostContent data={result.data} />
    </ChartCard>
  );
}

interface ContentProps {
  data: TokenUsageTopIssuesByCostResponse | undefined;
}

function TopIssuesByCostContent({ data }: ContentProps) {
  const issues = data?.issues ?? [];
  if (issues.length === 0) {
    return <div className={styles.empty}>No data in this window</div>;
  }
  return (
    <ol className={styles.topIssuesList} data-testid="top-issues-by-cost-content">
      {issues.map((issue, idx) => {
        const count = Number(issue.session_count);
        return (
          <li
            key={issue.issue_id}
            className={styles.topIssuesRow}
            data-testid={`top-issues-row-${issue.issue_id}`}
          >
            <span className={styles.topIssuesRank}>{idx + 1}.</span>
            <Link
              to={`/issues/${issue.issue_id}`}
              className={styles.topIssuesTitle}
              title={issue.title}
            >
              {issue.title}
            </Link>
            <span className={styles.topIssuesCost}>{formatUsd(issue.cost_usd)}</span>
            <span className={styles.topIssuesCount}>
              {count === 1 ? "1 session" : `${count} sessions`}
            </span>
          </li>
        );
      })}
    </ol>
  );
}
