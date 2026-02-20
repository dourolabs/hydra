import { useState } from "react";
import { Avatar, Badge, MarkdownViewer, Panel, Tabs } from "@metis/ui";
import type { IssueVersionRecord } from "@metis/api";
import { issueToBadgeStatus } from "../../utils/statusMapping";
import { IssueTodoList } from "./IssueTodoList";
import { IssueChildren } from "./IssueChildren";
import { IssueActivity } from "./IssueActivity";
import { JobList } from "../jobs/JobList";
import { PatchList } from "../patches/PatchList";
import styles from "./IssueDetail.module.css";

interface IssueDetailProps {
  record: IssueVersionRecord;
}

const TABS = [
  { id: "children", label: "Children" },
  { id: "jobs", label: "Jobs" },
  { id: "patches", label: "Patches" },
  { id: "todo", label: "Todo" },
  { id: "activity", label: "Activity" },
];

export function IssueDetail({ record }: IssueDetailProps) {
  const [activeTab, setActiveTab] = useState("children");
  const { issue } = record;

  return (
    <div className={styles.detail}>
      {/* Header: ID + Status */}
      <div className={styles.header}>
        <span className={styles.issueId}>{record.issue_id}</span>
        <Badge status={issueToBadgeStatus(issue.status)} />
        <span className={styles.type}>{issue.type}</span>
      </div>

      {/* Metadata */}
      <div className={styles.meta}>
        {issue.creator && (
          <div className={styles.metaItem}>
            <span className={styles.metaLabel}>Creator</span>
            <span className={styles.metaValue}>
              <Avatar name={issue.creator} size="sm" />
              {issue.creator}
            </span>
          </div>
        )}
        {issue.assignee && (
          <div className={styles.metaItem}>
            <span className={styles.metaLabel}>Assignee</span>
            <span className={styles.metaValue}>
              <Avatar name={issue.assignee} size="sm" />
              {issue.assignee}
            </span>
          </div>
        )}
        <div className={styles.metaItem}>
          <span className={styles.metaLabel}>Updated</span>
          <span className={styles.metaValue}>
            {new Date(record.timestamp).toLocaleString()}
          </span>
        </div>
      </div>

      {/* Description */}
      <Panel header={<span className={styles.sectionTitle}>Description</span>}>
        <div className={styles.sectionBody}>
          {issue.description ? (
            <MarkdownViewer content={issue.description} />
          ) : (
            <p className={styles.empty}>No description.</p>
          )}
        </div>
      </Panel>

      {/* Progress */}
      {issue.progress && (
        <Panel header={<span className={styles.sectionTitle}>Progress</span>}>
          <div className={styles.sectionBody}>
            <MarkdownViewer content={issue.progress} />
          </div>
        </Panel>
      )}

      {/* Tabbed sections: Children, Tasks, Patches, Todo */}
      <Panel
        header={
          <Tabs
            tabs={TABS}
            activeTab={activeTab}
            onTabChange={setActiveTab}
          />
        }
      >
        <div className={styles.sectionBody}>
          {activeTab === "children" && (
            <IssueChildren issueId={record.issue_id} />
          )}
          {activeTab === "jobs" && <JobList issueId={record.issue_id} />}
          {activeTab === "patches" && (
            <PatchList
              patchIds={issue.patches ?? []}
              issueId={record.issue_id}
            />
          )}
          {activeTab === "todo" && (
            <IssueTodoList items={issue.todo_list ?? []} />
          )}
          {activeTab === "activity" && (
            <IssueActivity issueId={record.issue_id} />
          )}
        </div>
      </Panel>
    </div>
  );
}
