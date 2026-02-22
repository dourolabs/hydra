import { useState, useMemo, useCallback, useEffect } from "react";
import { useSearchParams, useNavigate } from "react-router-dom";
import { Spinner, Tabs } from "@metis/ui";
import type { IssueVersionRecord } from "@metis/api";
import { useIssues } from "../features/issues/useIssues";
import { computeBlockedStatus } from "../features/issues/blockedStatus";
import { topologicalSort } from "../features/issues/topologicalSort";
import { useAllJobs } from "../features/jobs/useAllJobs";
import { useAuth } from "../features/auth/useAuth";
import { actorDisplayName } from "../api/auth";
import { SplitLayout } from "../layout/SplitLayout";
import { InboxList } from "../features/dashboard/InboxList";
import { WatchingTree } from "../features/dashboard/WatchingTree";
import { useWatchingCount } from "../features/dashboard/useWatchingCount";
import { DetailPanel, DetailPanelEmpty } from "../features/dashboard/DetailPanel";
import { IssueCreateModal } from "../features/dashboard/IssueCreateModal";
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
  const navigate = useNavigate();
  const [searchParams, setSearchParams] = useSearchParams();
  const selectedId = searchParams.get("selected");
  const activeTab = searchParams.get("tab") ?? "inbox";
  const [createModalOpen, setCreateModalOpen] = useState(false);

  const setSelectedId = useCallback(
    (id: string | null) => {
      // Push a history entry when going from no selection to a selection
      // so the browser back button can return to the list view.
      // Replace when switching between items or deselecting to avoid
      // cluttering the history stack.
      const shouldPush = id !== null && selectedId === null;
      setSearchParams(
        (prev) => {
          const next = new URLSearchParams(prev);
          if (id) {
            next.set("selected", id);
          } else {
            next.delete("selected");
          }
          return next;
        },
        { replace: !shouldPush },
      );
    },
    [setSearchParams, selectedId],
  );

  const setActiveTab = useCallback(
    (tab: string) => {
      // Push a history entry when actually changing tabs so back button
      // reverses tab switches. Replace if re-selecting the current tab.
      const shouldPush = tab !== activeTab;
      setSearchParams(
        (prev) => {
          const next = new URLSearchParams(prev);
          next.set("tab", tab);
          return next;
        },
        { replace: !shouldPush },
      );
    },
    [setSearchParams, activeTab],
  );

  const handleMobileBack = useCallback(() => {
    navigate(-1);
  }, [navigate]);

  const username = user ? actorDisplayName(user.actor) : "";

  const inboxIssues = useMemo(() => {
    if (!issues) return [];
    const issueMap = new Map<string, IssueVersionRecord>();
    for (const record of issues) {
      issueMap.set(record.issue_id, record);
    }
    const filtered = issues
      .filter((i) => isInbox(i, username) && !computeBlockedStatus(i, issueMap).blocked)
      .sort(
        (a, b) =>
          new Date(b.timestamp).getTime() - new Date(a.timestamp).getTime(),
      );
    return topologicalSort(filtered);
  }, [issues, username]);

  const assignees = useMemo(() => {
    if (!issues) return [];
    const set = new Set<string>();
    for (const record of issues) {
      if (record.issue.assignee) set.add(record.issue.assignee);
    }
    return Array.from(set).sort();
  }, [issues]);

  const selectedRecord = useMemo(
    () => issues?.find((i) => i.issue_id === selectedId) ?? null,
    [issues, selectedId],
  );

  useEffect(() => {
    if (selectedId && issues && !selectedRecord) {
      setSelectedId(null);
    }
  }, [selectedId, issues, selectedRecord, setSelectedId]);

  const watchingCount = useWatchingCount(issues, jobsByIssue);

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
          username={username}
        />
      )}
      <button
        type="button"
        className={styles.createButton}
        onClick={() => setCreateModalOpen(true)}
      >
        + Create Issue
      </button>
    </div>
  );

  const rightPane = selectedRecord ? (
    <DetailPanel record={selectedRecord} />
  ) : (
    <DetailPanelEmpty />
  );

  return (
    <div className={styles.page}>
      <SplitLayout
        left={leftPane}
        right={rightPane}
        leftWidth={40}
        mobileDetailVisible={selectedId !== null}
        onMobileBack={handleMobileBack}
      />
      <IssueCreateModal
        open={createModalOpen}
        onClose={() => setCreateModalOpen(false)}
        assignees={assignees}
      />
    </div>
  );
}
