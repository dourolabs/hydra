import { useState, useMemo } from "react";
import { Spinner, Tabs } from "@metis/ui";
import type { IssueVersionRecord } from "@metis/api";
import { useIssues } from "../features/issues/useIssues";
import { useAllJobs } from "../features/jobs/useAllJobs";
import { useAuth } from "../features/auth/useAuth";
import { actorDisplayName } from "../api/auth";
import { SplitLayout } from "../layout/SplitLayout";
import { InboxList } from "../features/dashboard/InboxList";
import { WatchingTree, useWatchingCount } from "../features/dashboard/WatchingTree";
import { DetailPanel, DetailPanelEmpty } from "../features/dashboard/DetailPanel";
import styles from "./DashboardPage.module.css";

function isInbox(record: IssueVersionRecord, username: string): boolean {
  return (
    record.issue.assignee === username &&
    (record.issue.status === "open" || record.issue.status === "in-progress")
  );
}

export function DashboardPage() {
  const { user } = useAuth();
  const { data: issues, isLoading } = useIssues();
  const { data: jobsByIssue } = useAllJobs();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState("inbox");

  const username = user ? actorDisplayName(user.actor) : "";

  const inboxIssues = useMemo(() => {
    if (!issues) return [];
    return issues
      .filter((i) => isInbox(i, username))
      .sort(
        (a, b) =>
          new Date(b.timestamp).getTime() - new Date(a.timestamp).getTime(),
      );
  }, [issues, username]);

  const selectedRecord = useMemo(
    () => issues?.find((i) => i.issue_id === selectedId) ?? null,
    [issues, selectedId],
  );

  const watchingCount = useWatchingCount(issues);

  const tabs = useMemo(
    () => [
      { id: "inbox", label: `Inbox (${inboxIssues.length})` },
      { id: "watching", label: `Watching (${watchingCount})` },
    ],
    [inboxIssues.length, watchingCount],
  );

  if (isLoading) {
    return (
      <div className={styles.center}>
        <Spinner size="lg" />
      </div>
    );
  }

  const leftPane = (
    <div className={styles.leftPane}>
      <Tabs
        tabs={tabs}
        activeTab={activeTab}
        onTabChange={setActiveTab}
        className={styles.tabs}
      />
      {activeTab === "inbox" && (
        <InboxList
          issues={inboxIssues}
          selectedId={selectedId}
          onSelect={setSelectedId}
        />
      )}
      {activeTab === "watching" && (
        <WatchingTree
          issues={issues ?? []}
          jobsByIssue={jobsByIssue ?? new Map()}
          selectedId={selectedId}
          onSelect={setSelectedId}
        />
      )}
    </div>
  );

  const rightPane = selectedRecord ? (
    <DetailPanel record={selectedRecord} />
  ) : (
    <DetailPanelEmpty />
  );

  return (
    <div className={styles.page}>
      <SplitLayout left={leftPane} right={rightPane} leftWidth={40} />
    </div>
  );
}
