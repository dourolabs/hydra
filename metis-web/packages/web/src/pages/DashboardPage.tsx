import { Panel, Spinner } from "@metis/ui";
import { useIssues } from "../features/issues/useIssues";
import { IssueTree } from "../features/issues/IssueTree";
import styles from "./DashboardPage.module.css";

export function DashboardPage() {
  const { data: issues, isLoading, error } = useIssues();

  return (
    <div className={styles.page}>
      <Panel header={<span className={styles.header}>Issues</span>}>
        {isLoading && (
          <div className={styles.center}>
            <Spinner size="md" />
          </div>
        )}
        {error && (
          <p className={styles.error}>Failed to load issues: {(error as Error).message}</p>
        )}
        {issues && issues.length === 0 && (
          <p className={styles.empty}>No issues found.</p>
        )}
        {issues && issues.length > 0 && <IssueTree issues={issues} />}
      </Panel>
    </div>
  );
}
