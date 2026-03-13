import { useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { Avatar, Badge, MarkdownViewer, Panel, Tabs } from "@metis/ui";
import type { IssueVersionRecord } from "@metis/api";
import { normalizeIssueStatus } from "../../utils/statusMapping";
import { formatTimestamp } from "../../utils/time";
import { useIssue } from "./useIssue";
import { IssueRelatedIssues } from "./IssueRelatedIssues";
import { IssueActivity } from "./IssueActivity";
import { IssueUpdateModal } from "./IssueUpdateModal";
import { SessionList } from "../sessions/SessionList";
import { PatchList } from "../patches/PatchList";
import { PatchPreview } from "./PatchPreview";
import { DocumentPreview } from "./DocumentPreview";
import { extractDocumentPaths } from "../dashboard/useTransitiveWorkItems";
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
          <Badge status={normalizeIssueStatus(record.issue.status)} />
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
  { id: "sessions", label: "Sessions" },
  { id: "patches", label: "Patches" },
  { id: "activity", label: "Activity" },
  { id: "metadata", label: "Metadata" },
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

  const documentPaths = useMemo(() => {
    const texts = [issue.description, issue.progress].filter(Boolean) as string[];
    const allPaths = new Set<string>();
    for (const text of texts) {
      for (const path of extractDocumentPaths(text)) {
        allPaths.add(path);
      }
    }
    return Array.from(allPaths);
  }, [issue.description, issue.progress]);

  return (
    <div className={styles.detail}>
      {/* Header: Title + Status */}
      <div className={styles.header}>
        <div className={styles.headerLeft}>
          <h1 className={styles.issueTitle}>
            {issue.title || record.issue_id}
          </h1>
          <div className={styles.subtitle}>
            <span className={styles.type}>{issue.type}</span>
          </div>
        </div>
        <button
          type="button"
          className={styles.statusChip}
          data-testid="status-chip"
          onClick={() => setUpdateModalOpen(true)}
        >
          <Badge status={normalizeIssueStatus(issue.status)} />
          <span className={styles.statusChipIcon}>
            <svg viewBox="0 0 20 20" fill="currentColor">
              <path fillRule="evenodd" d="M5.293 7.293a1 1 0 011.414 0L10 10.586l3.293-3.293a1 1 0 111.414 1.414l-4 4a1 1 0 01-1.414 0l-4-4a1 1 0 010-1.414z" clipRule="evenodd" />
            </svg>
          </span>
        </button>
      </div>

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

      {/* Patch Preview */}
      {(issue.patches ?? []).length > 0 && (
        <PatchPreview
          patchIds={issue.patches ?? []}
          issueId={record.issue_id}
        />
      )}

      {/* Document Preview */}
      {documentPaths.length > 0 && (
        <DocumentPreview paths={documentPaths} />
      )}

      {/* Progress */}
      {issue.progress && (
        <Panel header={<span className={styles.sectionTitle}>Progress</span>}>
          <div className={styles.progressBody}>
            <MarkdownViewer content={issue.progress} />
          </div>
        </Panel>
      )}

      {/* Tabbed sections: Related Issues, Sessions, Patches, Activity, Metadata */}
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
          {activeTab === "sessions" && <SessionList issueId={record.issue_id} />}
          {activeTab === "patches" && (
            <PatchList
              patchIds={issue.patches ?? []}
              issueId={record.issue_id}
            />
          )}
          {activeTab === "activity" && (
            <IssueActivity issueId={record.issue_id} />
          )}
          {activeTab === "metadata" && (
            <div className={styles.metadataTab}>
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
              <IssueSettings jobSettings={issue.session_settings} />
            </div>
          )}
        </div>
      </Panel>
    </div>
  );
}
