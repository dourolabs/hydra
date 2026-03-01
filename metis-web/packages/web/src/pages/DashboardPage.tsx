import { useState, useMemo, useCallback } from "react";
import { Spinner } from "@metis/ui";
import { useIssues, buildIssueTree } from "../features/issues/useIssues";
import { useAllJobs } from "../features/jobs/useAllJobs";
import { useAuth } from "../features/auth/useAuth";
import { actorDisplayName } from "../api/auth";
import { IssueFilterSidebar } from "../features/dashboard/IssueFilterSidebar";
import { HeterogeneousItemList } from "../features/dashboard/HeterogeneousItemList";
import { useTransitiveWorkItems } from "../features/dashboard/useTransitiveWorkItems";
import { readCollapsed, writeCollapsed } from "../features/dashboard/sidebarStorage";
import { IssueCreateModal } from "../features/dashboard/IssueCreateModal";
import styles from "./DashboardPage.module.css";

export function DashboardPage() {
  const { user } = useAuth();
  const { data: issues, isLoading } = useIssues();
  const { data: jobsByIssue } = useAllJobs();
  const [createModalOpen, setCreateModalOpen] = useState(false);
  const [filterRootId, setFilterRootId] = useState<string | null>(null);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(readCollapsed);
  const [drawerOpen, setDrawerOpen] = useState(false);

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

  const { items: workItems, isLoading: workItemsLoading } =
    useTransitiveWorkItems(filterRootId, issues ?? []);

  const handleToggleSidebar = useCallback(() => {
    const next = !sidebarCollapsed;
    writeCollapsed(next);
    setSidebarCollapsed(next);
  }, [sidebarCollapsed]);

  const handleToggleDrawer = useCallback(() => {
    setDrawerOpen((v) => !v);
  }, []);

  const handleDrawerClose = useCallback(() => {
    setDrawerOpen(false);
  }, []);

  if (isLoading) {
    return (
      <div className={styles.center}>
        <Spinner size="lg" />
      </div>
    );
  }

  return (
    <div className={styles.page}>
      <div className={styles.dashboardRow}>
        <IssueFilterSidebar
          roots={roots}
          activeFilter={filterRootId}
          onFilterChange={setFilterRootId}
          collapsed={sidebarCollapsed}
          drawerOpen={drawerOpen}
          onDrawerClose={handleDrawerClose}
          jobsByIssue={jobsByIssue ?? new Map()}
          username={username}
        />
        <HeterogeneousItemList
          items={workItems}
          jobsByIssue={jobsByIssue ?? new Map()}
          isLoading={workItemsLoading}
          sidebarCollapsed={sidebarCollapsed}
          onToggleSidebar={handleToggleSidebar}
          onToggleDrawer={handleToggleDrawer}
        />
      </div>
      <button
        type="button"
        className={styles.createButton}
        onClick={() => setCreateModalOpen(true)}
      >
        + Create Issue
      </button>
      <IssueCreateModal
        open={createModalOpen}
        onClose={() => setCreateModalOpen(false)}
        assignees={assignees}
      />
    </div>
  );
}
