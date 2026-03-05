import { useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { Avatar, Badge, Button, MarkdownViewer, Panel, Tabs } from "@metis/ui";
import type { IssueVersionRecord } from "@metis/api";
import { issueToBadgeStatus } from "../../utils/statusMapping";
import { formatTimestamp } from "../../utils/time";
import { useIssue } from "./useIssue";
import { IssueTodoList } from "./IssueTodoList";
import { IssueRelatedIssues } from "./IssueRelatedIssues";
import { IssueActivity } from "./IssueActivity";
import { IssueUpdateModal } from "./IssueUpdateModal";
import { JobList } from "../jobs/JobList";
import { PatchList } from "../patches/PatchList";
import { IssueSettings } from "./IssueSettings";
import { IssueLabelEditor } from "./IssueLabelEditor";
import styles from "./IssueDetail.module.css";

function BlockingIssueLink({ issueId }: { issueId: string }) {
  const { data: record } = useIssue(issueId);
  return (
    <span className={styles.blockingIssue}>
      <Link to={`/issues/${issueId}`} className={styles.blockingIssueLink}>
        {issueId}
      </Link>
      {record && (
        <>
          <Badge status={issueToBadgeStatus(record.issue.status)} />
          <span className={styles.blockingIssueStatus}>({record.issue.status})</span>
        </>
      )}
    </span>
  );
}

interface IssueDetailProps {
  record: IssueVersionRecord;
}

const TABS = [
  { id: "related", label: "Related Issues" },
  { id: "jobs", label: "Jobs" },
  { id: "patches", label: "Patches" },
  { id: "todo", label: "Todo" },
  { id: "activity", label: "Activity" },
  { id: "settings", label: "Settings" },
];

export function IssueDetail({ record }: IssueDetailProps) {
  const [activeTab, setActiveTab] = useState("related");
  const [updateModalOpen, setUpdateModalOpen] = useState(false);
  const { issue } = record;

  const blockedOnIds = useMemo(
    () =>
      issue.dependencies
        .filter((d) => d.type === "blocked-on")
        .map((d) => d.issue_id),
    [issue.dependencies],
  );

  return (
    <div className={styles.detail}>
      {/* Header: ID + Status */}
      <div className={styles.header}>
        <span className={styles.issueId}>{record.issue_id}</span>
        <Badge status={issueToBadgeStatus(issue.status)} />
        <span className={styles.type}>{issue.type}</span>
        <Button
          variant="secondary"
          size="sm"
          className={styles.updateStatusBtn}
          onClick={() => setUpdateModalOpen(true)}
        >
          Update Status
        </Button>
      </div>

      {issue.title && (
        <h1 className={styles.issueTitle}>{issue.title}</h1>
      )}

      <IssueUpdateModal
        open={updateModalOpen}
        onClose={() => setUpdateModalOpen(false)}
        issueId={record.issue_id}
        issue={issue}
      />

      {/* Blocked-by banner */}
      {blockedOnIds.length > 0 && (
        <div className={styles.blockedBanner}>
          <span className={styles.blockedBannerIcon}>⚠</span>
          <span className={styles.blockedBannerLabel}>Blocked by:</span>
          {blockedOnIds.map((id, idx) => (
            <span key={id}>
              {idx > 0 && <span className={styles.blockedSeparator}>·</span>}
              <BlockingIssueLink issueId={id} />
            </span>
          ))}
        </div>
      )}

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
          <span className={styles.metaLabel}>Created</span>
          <span className={styles.metaValue}>
            {formatTimestamp(record.creation_time)}
          </span>
        </div>
        <div className={styles.metaItem}>
          <span className={styles.metaLabel}>Updated</span>
          <span className={styles.metaValue}>
            {formatTimestamp(record.timestamp)}
          </span>
        </div>
      </div>

      {/* Labels */}
      <IssueLabelEditor
        issueId={record.issue_id}
        labels={record.labels ?? []}
      />

      {/* Description */}
      <div className={styles.description}>
        {issue.description ? (
          <MarkdownViewer content={issue.description} />
        ) : (
          <p className={styles.empty}>No description.</p>
        )}
      </div>

      {/* Progress */}
      {issue.progress && (
        <Panel header={<span className={styles.sectionTitle}>Progress</span>}>
          <div className={styles.progressBody}>
            <MarkdownViewer content={issue.progress} />
          </div>
        </Panel>
      )}

      {/* Tabbed sections: Related Issues, Jobs, Patches, Todo, Activity, Settings */}
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
          {activeTab === "related" && (
            <IssueRelatedIssues issueId={record.issue_id} />
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
          {activeTab === "settings" && (
            <IssueSettings jobSettings={issue.job_settings} />
          )}
        </div>
      </Panel>
    </div>
  );
}
