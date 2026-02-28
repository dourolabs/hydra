import { useState, useMemo, useCallback, useEffect, useRef } from "react";
import { useSearchParams, useNavigate } from "react-router-dom";
import { Spinner } from "@metis/ui";
import { useIssues, buildIssueTree } from "../features/issues/useIssues";
import { useAllJobs } from "../features/jobs/useAllJobs";
import { useAuth } from "../features/auth/useAuth";
import { actorDisplayName } from "../api/auth";
import { SplitLayout } from "../layout/SplitLayout";
import { WatchlistActivityFeed } from "../features/dashboard/WatchlistActivityFeed";
import { IssueFilterSidebar } from "../features/dashboard/IssueFilterSidebar";
import { readCollapsed } from "../features/dashboard/sidebarStorage";
import { DetailPanel, DetailPanelEmpty } from "../features/dashboard/DetailPanel";
import { IssueCreateModal } from "../features/dashboard/IssueCreateModal";
import styles from "./DashboardPage.module.css";

export function DashboardPage() {
  const { user } = useAuth();
  const { data: issues, isLoading } = useIssues();
  const { data: jobsByIssue } = useAllJobs();
  const navigate = useNavigate();
  const [searchParams, setSearchParams] = useSearchParams();
  const selectedId = searchParams.get("selected");
  const [createModalOpen, setCreateModalOpen] = useState(false);
  const [filterRootId, setFilterRootId] = useState<string | null>(null);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(readCollapsed);
  const hasSelectionHistoryRef = useRef(false);

  const setSelectedId = useCallback(
    (id: string | null) => {
      const shouldPush = id !== null && selectedId === null;
      if (shouldPush) {
        hasSelectionHistoryRef.current = true;
      }
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

  const handleMobileBack = useCallback(() => {
    if (hasSelectionHistoryRef.current) {
      hasSelectionHistoryRef.current = false;
      navigate(-1);
    } else {
      setSelectedId(null);
    }
  }, [navigate, setSelectedId]);

  const username = user ? actorDisplayName(user.actor) : "";

  const roots = useMemo(() => {
    if (!issues) return [];
    const tree = buildIssueTree(issues);
    return tree
      .filter(
        (root) =>
          !root.hardBlocked && root.issue.issue.creator === username,
      )
      .sort(
        (a, b) =>
          new Date(b.issue.creation_time).getTime() -
          new Date(a.issue.creation_time).getTime(),
      );
  }, [issues, username]);

  const assignees = useMemo(() => {
    if (!issues) return [];
    const set = new Set<string>();
    for (const record of issues) {
      if (record.issue.assignee) set.add(record.issue.assignee);
    }
    return Array.from(set).sort();
  }, [issues]);

  const selectedExists = useMemo(
    () => selectedId != null && issues?.some((i) => i.issue_id === selectedId),
    [issues, selectedId],
  );

  useEffect(() => {
    if (selectedId && issues && !selectedExists) {
      setSelectedId(null);
    }
  }, [selectedId, issues, selectedExists, setSelectedId]);

  if (isLoading) {
    return (
      <div className={styles.center}>
        <Spinner size="lg" />
      </div>
    );
  }

  const leftPane = (
    <div className={styles.leftPane}>
      <WatchlistActivityFeed
        issues={issues ?? []}
        jobsByIssue={jobsByIssue ?? new Map()}
        selectedId={selectedId}
        onSelect={setSelectedId}
        username={username}
        filterRootId={filterRootId}
      />
      <button
        type="button"
        className={styles.createButton}
        onClick={() => setCreateModalOpen(true)}
      >
        + Create Issue
      </button>
    </div>
  );

  const rightPane = selectedId && selectedExists ? (
    <DetailPanel issueId={selectedId} />
  ) : (
    <DetailPanelEmpty />
  );

  return (
    <div className={styles.page}>
      <div className={styles.dashboardRow}>
        <IssueFilterSidebar
          roots={roots}
          activeFilter={filterRootId}
          onFilterChange={setFilterRootId}
          collapsed={sidebarCollapsed}
          onToggleCollapsed={setSidebarCollapsed}
          jobsByIssue={jobsByIssue ?? new Map()}
        />
        <SplitLayout
          left={leftPane}
          right={rightPane}
          leftWidth={40}
          mobileDetailVisible={selectedId !== null}
          onMobileBack={handleMobileBack}
        />
      </div>
      <IssueCreateModal
        open={createModalOpen}
        onClose={() => setCreateModalOpen(false)}
        assignees={assignees}
      />
    </div>
  );
}
